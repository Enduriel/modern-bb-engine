//! Input arrives whenever the player moves the mouse or presses a key, not only when a
//! test sends it: at the main menu before any map exists, while a campaign loads and
//! the world is generated, on the world map, while a battle loads, in battle. The
//! windowed game once panicked on the first mouse move at the main menu (the world's
//! hover listener on a 0×0 map). Each phase here gets mouse moves (on and off the screen),
//! right clicks and key presses, and must take them without an error.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};
use squirrel_bb::{KeyInput, MouseInput};

/// Screen positions: corners, the middle, and well off the screen both ways.
const POSITIONS: [(i64, i64); 7] = [(0, 0), (1, 1), (640, 360), (1279, 719), (-500, -500), (100_000, 100_000), (2560, 1440)];

/// Keys the scripts see (digits, letters, Tab, Space, navigation, arrows, numpad,
/// modifiers); not Escape, Enter or the function keys, which leave the screen under test.
fn keys() -> impl Iterator<Item = i64> {
	(1..=36).chain([38, 40, 44, 45, 46, 47, 48, 49, 50, 51, 53, 54]).chain(55..=70).chain(95..=97)
}

/// Mouse moves and right clicks at every position, then each key pressed and released.
fn input(engine: &harness::GameEngine, phase: &str) {
	let vm = engine.vm();
	let state = engine.root_state();
	for (x, y) in POSITIONS {
		for event in [MouseInput::moved(x, y), MouseInput::right_up(x, y)] {
			state.mouse_input(vm, event).unwrap_or_else(|e| panic!("{phase}: mouse {event:?}: {e}"));
		}
	}
	for key in keys() {
		for event in [KeyInput::pressed(key), KeyInput::released(key)] {
			state.key_input(vm, event).unwrap_or_else(|e| panic!("{phase}: key {event:?}: {e}"));
		}
	}
}

fn frame(engine: &harness::GameEngine, phase: &str) {
	harness::run_frame(engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("{phase}: frame: {e}"));
}

/// Run frames (with input each) until `cond` holds; panics after `max`.
fn until(engine: &harness::GameEngine, max: u32, phase: &str, cond: &str) {
	for _ in 0..max {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap_or_else(|e| panic!("{phase}: {e}")) == Some(1) {
			return;
		}
		input(engine, phase);
		frame(engine, phase);
	}
	panic!("{phase}: still not true after {max} frames: {cond}");
}

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";

#[test]
fn input_at_any_time_is_taken() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
		return;
	}
	let mut engine = harness::open_install(dir).expect("open install");
	engine.vm_mut().install_standard_error_handlers();

	// Before the menu has booted, and while it shows: no map exists.
	input(&engine, "before the menu");
	harness::boot_menu(&engine).expect("boot the menu");
	// The scripted player (it engages the battle below).
	harness::install_autoplay(&engine).expect("autoplay");
	input(&engine, "the menu booting");
	until(&engine, 120, "the main menu", &format!("{MENU}.isVisible() && !{MENU}.isAnimating()"));
	for _ in 0..30 {
		input(&engine, "the main menu");
		frame(&engine, "the main menu");
	}

	// A new campaign: the loading screen and the worldgen behind it.
	engine
		.run_script(&format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"), "new campaign")
		.expect("new campaign");
	frame(&engine, "the new campaign page");
	engine
		.run_script(
			&format!(
				"{MENU}.getNewCampaignMenuModule().onStartButtonPressed({{
					companyName = \"Input Test Company\", banner = \"banner_04\",
					combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
					ironman = false, explorationDifficulty = false,
					lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
					seed = \"rust-engine-test\", origin = \"scenario.early_access\"
				}});"
			),
			"start",
		)
		.expect("start");
	until(
		&engine,
		600,
		"the campaign loading",
		"(\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()",
	);

	// The world map.
	engine
		.run_script("::World.State.setAutoPause(false); ::World.State.setPause(false);", "unpause")
		.expect("unpause");
	for _ in 0..60 {
		input(&engine, "the world map");
		frame(&engine, "the world map");
	}

	// A battle: its loading, then the battle.
	harness::engage_nearest_hostile(&engine).expect("engage");
	until(&engine, 3000, "the battle loading", "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible()");
	for _ in 0..120 {
		input(&engine, "the battle");
		frame(&engine, "the battle");
	}
}
