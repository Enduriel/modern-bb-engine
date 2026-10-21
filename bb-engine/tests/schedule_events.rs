//! Which scheduled `Time` events survive what, as in the original (probed and
//! disassembled 2026-10-20): the engine itself never drops an event. Only
//! `Time.clearEvents()` does, and the states' scripts call it when they leave a campaign
//! (`world_state.onFinish`), before a load (`onBeforeDeserialize`) and when a battle ends
//! (`tactical_state.onFinish`, fleeing). So:
//! - an event scheduled at the main menu lives on into a new campaign;
//! - quitting a campaign to the menu drops every pending event, of every unit;
//! - `World.clearScene()` and `Tactical.clearScene()` by themselves drop nothing.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
const AT_MENU: &str = "::RootState.get(\"WorldState\") == null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible() && !::RootState.get(\"MainMenuState\").m.MainMenuScreen.isAnimating()";
const LOADED: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";

fn frame(engine: &harness::GameEngine) {
	harness::run_frame(engine, 1000.0 / 60.0).unwrap();
}

fn until(engine: &harness::GameEngine, max: u32, what: &str, cond: &str) {
	for _ in 0..max {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap() == Some(1) {
			return;
		}
		frame(engine);
	}
	panic!("{what}: still not true after {max} frames: {cond}");
}

fn fired(engine: &harness::GameEngine) -> String {
	player::eval_string(engine, "::__fired.len() ? ::__fired.reduce(@(a, b) a + \",\" + b) : \"\"").unwrap().unwrap_or_default()
}

/// One event of each unit, tagged `tag_<unit>`, due after `ms` (Real/Virtual) or the next
/// round.
fn schedule_all(engine: &harness::GameEngine, tag: &str, ms: u32) {
	engine
		.run_script(
			&format!(
				r#"
				foreach (u in ["Real", "Virtual", "Rounds"])
				{{
					local name = "{tag}_" + u;
					::Time.scheduleEvent(::TimeUnit[u], u == "Rounds" ? 1 : {ms}, function( _d ) {{ ::__fired.push(_d); }}, name);
				}}
				"#
			),
			"schedule",
		)
		.unwrap();
}

#[test]
fn scheduled_events_are_dropped_only_by_clear_events() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
		return;
	}
	let mut engine = harness::open_install(dir).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	harness::install_autoplay(&engine).expect("autoplay");
	until(&engine, 600, "the main menu", AT_MENU);
	engine.run_script("::__fired <- [];", "log").unwrap();

	// The scene clears alone keep every event.
	let before = engine.time().pending_events();
	schedule_all(&engine, "menu", 200);
	engine.run_script("::Tactical.clearScene(); ::World.clearScene(); ::Time.setRound(::Time.getRound() + 1);", "clear the scenes").unwrap();
	assert_eq!(engine.time().pending_events(), before + 3, "the scene clears drop nothing");
	for _ in 0..30 {
		frame(&engine);
	}
	assert_eq!(fired(&engine), "menu_Rounds,menu_Real,menu_Virtual", "all three fire after the scene clears (the round at once, the others 200 ms on)");

	// A menu event lives on into a new campaign.
	engine.run_script("::Time.scheduleEvent(::TimeUnit.Real, 5000, function( _d ) { ::__fired.push(_d); }, \"from_menu\");", "menu event").unwrap();
	engine.run_script(&format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"), "new campaign").unwrap();
	frame(&engine);
	engine
		.run_script(
			&format!(
				"{MENU}.getNewCampaignMenuModule().onStartButtonPressed({{
					companyName = \"Events\", banner = \"banner_04\",
					combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
					ironman = false, explorationDifficulty = false,
					lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
					seed = \"rust-engine-test\", origin = \"scenario.early_access\"
				}});"
			),
			"start",
		)
		.unwrap();
	until(&engine, 600, "the campaign loading", LOADED);
	until(&engine, 600, "the menu's event firing in the campaign", "::__fired.find(\"from_menu\") != null");

	// Quitting drops every pending event, of every unit.
	schedule_all(&engine, "world", 60000);
	assert!(engine.time().pending_events() >= 3);
	engine.run_script("::World.State.exitGame();", "quit to the menu").unwrap();
	until(&engine, 600, "back at the menu", AT_MENU);
	engine.run_script("::Time.setRound(::Time.getRound() + 1);", "next round").unwrap();
	for _ in 0..10 {
		frame(&engine);
	}
	assert!(!fired(&engine).contains("world_"), "no campaign event fired after quitting: {}", fired(&engine));
	assert_eq!(engine.time().pending_events(), 0, "quitting dropped every event");
}
