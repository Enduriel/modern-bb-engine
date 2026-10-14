//! A second campaign in one run starts as clean as the first: campaign 1 → quit to the
//! main menu → campaign 2 with the same seed. Back at the menu, nothing of campaign 1
//! remains (no world entity, thinker, unit or roster, no scheduled event, no background
//! task; the same states and UI connections as before it). Campaign 2's world is
//! campaign 1's, entity for entity, and a frame of it costs what a frame of campaign 1
//! did.
//!
//! `World.clearScene()` (the world state's `onFinish`) once cleared only the clock: every
//! campaign's entities stayed and kept thinking, so each new campaign in a run was
//! slower than the last (the windowed game's hitches on the second campaign).
//!
//! Skips without the Steam install.

use std::path::Path;
use std::time::Instant;

use bb_engine::{harness, tactical_player as player};

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
const AT_MENU: &str = "::RootState.get(\"WorldState\") == null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible() && !::RootState.get(\"MainMenuState\").m.MainMenuScreen.isAnimating()";
const LOADED: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";
/// The world the seed made: its settlements, where they stand, and how many entities the
/// scripts see.
const WORLD: &str = r#"(function() {
	local s = "";
	foreach (e in ::World.EntityManager.getSettlements()) s += e.getName() + "@" + e.getTile().ID + " ";
	return s + ::World.getAllEntitiesAtPos(::createVec(0, 0), 10000000.0).len();
})()"#;

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

/// What the engine holds.
#[derive(Debug, PartialEq)]
struct Held {
	entities: usize,
	thinkers: usize,
	units: usize,
	events: usize,
	connections: usize,
	states: Vec<String>,
	background_task: bool,
}

fn held(engine: &harness::GameEngine) -> Held {
	Held {
		entities: engine.world_entities().count(),
		thinkers: engine.world_entities().thinker_count(),
		units: engine.tactical_entities().unit_count(),
		events: engine.time().pending_events(),
		connections: engine.ui().connection_count(),
		states: engine.root_state().state_names(),
		background_task: engine.root_state().background_callback().is_some(),
	}
}

/// A campaign through the menu, as a player starts one.
fn start(engine: &harness::GameEngine, n: u32) {
	engine.run_script(&format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"), "new campaign").unwrap();
	frame(engine);
	engine
		.run_script(
			&format!(
				"{MENU}.getNewCampaignMenuModule().onStartButtonPressed({{
					companyName = \"Company {n}\", banner = \"banner_04\",
					combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
					ironman = false, explorationDifficulty = false,
					lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
					seed = \"rust-engine-test\", origin = \"scenario.early_access\"
				}});"
			),
			"start",
		)
		.unwrap();
	until(engine, 600, "the campaign loading", LOADED);
}

/// The median frame, in ms, while the party walks a path.
fn walk(engine: &harness::GameEngine) -> f64 {
	engine.run_script("::World.State.setAutoPause(false); ::World.State.setPause(false);", "unpause").unwrap();
	for _ in 0..120 {
		frame(engine);
	}
	engine
		.run_script(
			r#"
			local player = ::World.State.getPlayer(), from = player.getTile();
			local nav = ::World.getNavigator(), settings = nav.createSettings();
			settings.ActionPointCosts = ::Const.World.TerrainTypeNavCost;
			settings.RoadMult = 1.0 / ::Const.World.MovementSettings.RoadMult;
			local best = null;
			foreach (d in [[6, 0], [-6, 0], [0, 6], [0, -6], [4, 4], [-4, -4]])
			{
				if (!::World.isValidTileSquare(from.SquareCoords.X + d[0], from.SquareCoords.Y + d[1])) continue;
				local to = ::World.getTileSquare(from.SquareCoords.X + d[0], from.SquareCoords.Y + d[1]);
				local path = nav.findPath(from, to, settings, 0);
				if (!path.isEmpty() && (best == null || path.getSize() > best.getSize())) best = path;
			}
			player.setPath(best);
			"#,
			"path",
		)
		.unwrap();
	let mut times: Vec<f64> = (0..600)
		.map(|_| {
			let t = Instant::now();
			frame(engine);
			t.elapsed().as_secs_f64() * 1000.0
		})
		.collect();
	times.sort_by(f64::total_cmp);
	times[times.len() / 2]
}

fn quit(engine: &harness::GameEngine) {
	engine.run_script("::World.State.exitGame();", "quit to the menu").unwrap();
	until(engine, 600, "back at the menu", AT_MENU);
}

#[test]
fn a_second_campaign_starts_as_clean_as_the_first() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let mut engine = harness::open_install(dir).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	// The scripted player answers the world's events.
	harness::install_autoplay(&engine).expect("autoplay");
	until(&engine, 600, "the main menu", AT_MENU);
	let menu = held(&engine);

	start(&engine, 1);
	let (world_1, held_1) = (player::eval_string(&engine, WORLD).unwrap(), held(&engine));
	let frame_1 = walk(&engine);
	quit(&engine);
	assert_eq!(held(&engine), menu, "back at the menu after campaign 1");

	start(&engine, 2);
	let (world_2, held_2) = (player::eval_string(&engine, WORLD).unwrap(), held(&engine));
	assert_eq!(world_2, world_1, "the seed's world, as campaign 1 had it");
	assert_eq!(held_2, held_1, "campaign 2 holds what campaign 1 did");
	let frame_2 = walk(&engine);
	eprintln!("median frame: campaign 1 {frame_1:.2} ms, campaign 2 {frame_2:.2} ms");
	// Lenient (the machine may be busy): campaign 1's leftovers doubled the cost.
	assert!(frame_2 < frame_1 * 1.5 + 0.2, "a frame of campaign 2 costs {frame_2:.2} ms, of campaign 1 {frame_1:.2} ms");
	quit(&engine);
	assert_eq!(held(&engine), menu, "back at the menu after campaign 2");
}
