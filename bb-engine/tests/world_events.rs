//! World events that hand the company people: they are made in the world's temporary
//! roster (`World.getTemporaryRoster`, its own roster, not the battle's) and joined from
//! there. Here the deserter in the forest: hiding him creates him in the temporary
//! roster, welcoming him moves him into the company and clears it. The event runs
//! through its screen's buttons, as a player clicks them.
//!
//! Skips if no Steam install. Slow in debug builds: it runs all of worldgen first.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

fn count(engine: &harness::GameEngine, expr: &str) -> i64 {
	player::eval_i64(engine, expr).unwrap_or_else(|e| panic!("{expr}: {e}")).expect(expr)
}

#[test]
fn an_event_recruit_goes_through_the_world_temporary_roster() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	for _ in 0..10 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	const COMPANY: &str = "::World.getPlayerRoster().getSize()";
	const TEMPORARY: &str = "::World.getTemporaryRoster().getSize()";
	let company = count(&engine, COMPANY);
	let fired = count(&engine, "::World.Events.fire(\"event.deserter_in_forest\") ? 1 : 0");
	assert_eq!(fired, 1, "the event didn't show");
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");

	// "What? Who? Where?": hide him. Screen B makes him in the temporary roster.
	engine.run_script("::World.State.m.EventScreen.onButtonPressed(1);", "hide him").expect("hide him");
	assert_eq!(count(&engine, TEMPORARY), 1, "the deserter isn't in the temporary roster");
	assert_eq!(count(&engine, "::Tactical.getTemporaryRoster().getSize()"), 0, "the battle's temporary roster was used");
	assert_eq!(count(&engine, COMPANY), company);

	// "Welcome to the company."
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	engine.run_script("::World.State.m.EventScreen.onButtonPressed(0);", "welcome him").expect("welcome him");
	assert_eq!(count(&engine, COMPANY), company + 1, "he didn't join");
	assert_eq!(count(&engine, TEMPORARY), 0, "the temporary roster wasn't cleared");
}
