//! End-to-end: a battle from the campaign map, headless. A new campaign runs briefly,
//! the player engages the nearest hostile party through the real flow (combat dialog,
//! loading screen, the world state starting the combat), and the battle is fought out
//! frame by frame: the turn order bar's UI is emulated, the AI moves and attacks, the
//! scripted player ends its units' turns. Checks that it ends with a result and the
//! game returns to the world map, all without a script error.
//!
//! Skips if no Steam install. Slow in debug builds: it runs all of worldgen first.

use std::path::Path;

use bb_engine::harness;
use squirrel::SqTable;

#[test]
fn a_battle_from_the_campaign_map_is_fought_out() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::unpause_world(&engine).expect("unpause");
	for frame in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
	}
	harness::engage_nearest_hostile(&engine).expect("engage");

	const TACTICAL: &str = "TacticalFromWorldState";
	let mut fought = false;
	for frame in 0..30_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("battle frame {frame}: {e}"));
		let states = engine.root_state().state_names();
		if states.iter().any(|s| s == TACTICAL) {
			fought = true;
		} else if fought {
			break;
		}
	}
	assert!(fought, "the battle never started");
	let states = engine.root_state().state_names();
	assert!(!states.iter().any(|s| s == TACTICAL), "the battle never ended: {states:?}");
	assert!(states.iter().any(|s| s == "WorldState"), "not back on the world map: {states:?}");
	let result = SqTable::root(engine.vm()).get::<i64>("__lastCombatResult");
	assert!(matches!(result, Some(r) if r != 0), "no combat result: {result:?}");
}
