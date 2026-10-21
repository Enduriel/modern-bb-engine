//! A battle between loads doesn't grow memory either (vanilla's leak may need a battle
//! between reloads or saves). Each cycle loads the original's save of a new campaign
//! (`harness::ORIGINAL_SAVES_DIR`, the same fixture as
//! `reload_memory`), fights the nearest hostile party to the end (the AI plays both sides),
//! goes back to the world, saves, and loads the fixture again.
//!
//! A battle leaves tens of thousands of script objects in reference cycles (its units' AI
//! agents and behaviours), which reference counting never frees, so they would pile up
//! battle after battle. The engine collects them where the game discards such graphs (a
//! battle's end, leaving a campaign, a load; squirrel-bb's `collect`). Measured right after
//! each load, with no collection of the test's own, the heap stays put over the cycles,
//! the engine holds the same entities, thinkers, units,
//! scheduled events and UI connections, and nothing waits in cycles. One test in its own
//! binary. Skips without the Steam install or the original's saves.

use std::path::Path;

use bb_engine::{harness, heap, tactical_player as player};

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
const AT_MENU: &str = "::RootState.get(\"WorldState\") == null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible() && !::RootState.get(\"MainMenuState\").m.MainMenuScreen.isAnimating()";
const LOADED: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";
/// Back on the world map after a battle: no tactical state, the world shown and settled.
const BACK_IN_WORLD: &str = "(!(\"State\" in ::Tactical) || ::Tactical.State == null) && ::World.isVisible() && !::LoadingScreen.isVisible() && (\"__lastCombatResult\" in getroottable())";

const WARM_UP: usize = 2;
const CYCLES: usize = 8;

fn frames(engine: &harness::GameEngine, n: u32) {
	for _ in 0..n {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap();
	}
}

fn until(engine: &harness::GameEngine, max: u32, what: &str, cond: &str) {
	for _ in 0..max {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap() == Some(1) {
			return;
		}
		frames(engine, 1);
	}
	panic!("{what}: still not true after {max} frames: {cond}");
}

#[derive(Debug, PartialEq)]
struct Held {
	entities: usize,
	thinkers: usize,
	units: usize,
	events: usize,
	connections: usize,
	states: Vec<String>,
}

fn held(engine: &harness::GameEngine) -> Held {
	Held {
		entities: engine.world_entities().count(),
		thinkers: engine.world_entities().thinker_count(),
		units: engine.tactical_entities().unit_count(),
		events: engine.time().pending_events(),
		connections: engine.ui().connection_count(),
		states: engine.root_state().state_names(),
	}
}

/// What only reference cycles keep alive, by kind: `typeof`, and for a BB object its class
/// (`ClassName`) or, for a table, a few of its keys.
fn cycle_summary(engine: &harness::GameEngine) -> String {
	let Some(objects) = engine.vm().unreachable() else { return "none".to_string() };
	let vm = engine.vm();
	let summarize = vm
		.compile(
			r#"return function( _a ) {
				local h = {};
				foreach (o in _a)
				{
					local k = typeof o;
					try
					{
					if (k == "table" || k == "instance")
					{
						if (k == "table" && "ClassName" in o) k += " " + o.ClassName;
						else if (k == "instance") k += " " + o.getclass().tostring();
						else
						{
							local keys = [];
							foreach (kk, v in o) { if (keys.len() < 4) keys.push(kk.tostring()); }
							keys.sort();
							k += " {" + (keys.len() ? keys.reduce(@(a, b) a + "," + b) : "") + "}";
						}
					}
					else if (k == "function")
					{
						local i = o.getinfos();
						k += " " + ("name" in i ? i.name : "?") + "@" + ("src" in i ? i.src : "?");
					}
					}
					catch (e) k += " (?)";
					h[k] <- (k in h ? h[k] : 0) + 1;
				}
				local rows = [];
				foreach (k, n in h) rows.push([n, k]);
				rows.sort(@(a, b) b[0] <=> a[0]);
				local s = "";
				foreach (r in rows) s += "\n    " + r[0] + " x " + r[1];
				return _a.len() + " objects:" + s;
			}"#,
			"cycle summary",
		)
		.expect("compile");
	let f = summarize.call_ret::<squirrel::SqStrongRef, _>(vm, ()).ok().flatten().expect("summary fn");
	f.call_ret::<String, _>(vm, (objects,)).ok().flatten().unwrap_or_default()
}

/// How many objects only reference cycles keep alive (what the next collection frees).
fn waiting_in_cycles(engine: &harness::GameEngine) -> usize {
	let Some(objects) = engine.vm().unreachable() else { return 0 };
	let vm = engine.vm();
	let len = vm.compile("return function( _a ) { return _a.len(); }", "len").expect("compile");
	let f = len.call_ret::<squirrel::SqStrongRef, _>(vm, ()).ok().flatten().expect("len fn");
	f.call_ret::<i64, _>(vm, (objects,)).ok().flatten().unwrap_or(0) as usize
}

#[test]
fn battles_between_loads_do_not_grow_memory() {
	let from = Path::new(harness::ORIGINAL_SAVES_DIR).join("new_campaign.sav");
	if !Path::new(harness::STEAM_DATA_DIR).exists() || !from.exists() {
		bb_engine::harness::skip(format!("needs the Steam install and {}", from.display()));
		return;
	}
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reload_combat_memory");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	std::fs::copy(&from, dir.join("bb_probe_reload.sav")).unwrap();

	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	engine.saves().set_directory(dir.clone());
	harness::boot_menu(&engine).expect("boot the menu");
	harness::install_autoplay(&engine).expect("autoplay");
	until(&engine, 600, "the main menu", AT_MENU);
	until(&engine, 600, "4 s at the menu", "::Time.getRealTimeF() > 4.5");
	engine.run_script(&format!("{MENU}.getMainMenuModule().onLoadCampaignButtonPressed();"), "load menu").unwrap();
	frames(&engine, 1);
	engine.run_script(&format!("{MENU}.getLoadCampaignMenuModule().onLoadButtonPressed(\"bb_probe_reload\");"), "load").unwrap();
	until(&engine, 600, "the campaign loading", LOADED);

	let mb = |b: usize| b as f64 / 1_048_576.0;
	let mut after: Vec<(usize, Held, usize)> = Vec::new();
	for cycle in 1..=CYCLES {
		// The AI fights for the player's units too, so the battle ends on its own.
		engine
			.run_script(
				"::__autoplayFight <- true; if (\"__lastCombatResult\" in getroottable()) delete ::__lastCombatResult;",
				"fight",
			)
			.unwrap();
		engine.run_script("::World.State.setAutoPause(false); ::World.State.setPause(false);", "unpause").unwrap();
		harness::engage_nearest_hostile(&engine).expect("engage");
		until(&engine, 60 * 600, "the battle over and back in the world", BACK_IN_WORLD);
		let result = player::eval_i64(&engine, "::__lastCombatResult").unwrap();
		engine.run_script("::World.State.saveCampaign(\"bb_probe_after_battle\");", "save").unwrap();
		// The world state ignores a load within 4 s of the last.
		frames(&engine, 300);
		engine.run_script("::World.State.loadCampaign(\"bb_probe_reload\");", "load").unwrap();
		frames(&engine, 1);
		let waiting = waiting_in_cycles(&engine);
		let h = held(&engine);
		eprintln!("cycle {cycle}: battle result {result:?}, heap {:.2} MB, {waiting} objects waiting in cycles, {h:?}", mb(heap::bytes_in_use()));
		after.push((heap::bytes_in_use(), h, waiting));
	}
	let measured = &after[WARM_UP..];
	let (base_heap, base_held, _) = &measured[0];
	for (i, (_, h, waiting)) in after.iter().enumerate() {
		assert_eq!(*waiting, 0, "cycle {}: objects left in reference cycles: {}", i + 1, cycle_summary(&engine));
		if i >= WARM_UP {
			assert_eq!(h, base_held, "cycle {}: the engine holds more after a battle", i + 1);
		}
	}
	let last = measured.last().unwrap().0;
	eprintln!("heap {:.2} → {:.2} MB over {} cycles", mb(*base_heap), mb(last), measured.len());
	assert!(last <= base_heap + 2 * 1_048_576, "the heap grew {:.2} MB over {} cycles", mb(last.saturating_sub(*base_heap)), measured.len());
	let _ = std::fs::remove_dir_all(&dir);
}
