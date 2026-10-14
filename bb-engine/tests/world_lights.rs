//! A settlement's lights: its `lighting` sprite, which its script keeps clear by day and
//! fades in at night (settlement.onUpdate). Coming into sight fades an entity's sprites
//! in, but not that one: the entity-level fades leave a sprite with
//! `setSpriteColorization(name, false)` alone, as the original's do. So a new campaign,
//! at dawn, shows its towns unlit (frame parity: ours were lit), and night lights them.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

/// How many active settlements have their lights showing (alpha above 0).
const LIT: &str = "(function() { local n = 0; foreach (s in ::World.EntityManager.getSettlements()) if (s.isActive() && s.getSprite(\"lighting\").Alpha > 0) n++; return n; })()";

#[test]
fn towns_are_unlit_by_day_and_lit_at_night() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let mut engine = harness::open_install(dir).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "bbuitest").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	// The campaign starts at dawn, paused; the towns come into sight.
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	assert_eq!(player::eval_i64(&engine, "::World.getTime().IsDaytime ? 1 : 0").unwrap(), Some(1), "it's day");
	assert_eq!(player::eval_i64(&engine, LIT).unwrap(), Some(0), "no town is lit by day");
	// Night: the towns light up as they think.
	engine
		.run_script("::World.State.m.MenuStack.popAll(true); ::World.State.setAutoPause(false); ::World.State.setPause(false); ::World.setSpeedMult(4.0);", "run")
		.expect("run");
	let mut frames = 0;
	while player::eval_i64(&engine, "::World.getTime().TimeOfDay >= 5 ? 1 : 0").unwrap() != Some(1) {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
		frames += 1;
		if frames % 60 == 0 {
			engine.run_script("if (::World.State.isPaused()) { ::World.State.setAutoPause(false); ::World.State.setPause(false); } ::World.setSpeedMult(4.0);", "keep running").expect("run");
		}
		assert!(frames < 20_000, "night never fell");
	}
	for _ in 0..600 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	let lit = player::eval_i64(&engine, LIT).unwrap().unwrap();
	eprintln!("towns lit at night: {lit}");
	assert!(lit > 0, "towns light up at night");
}
