//! A held key reaches the states every frame (the original's key queue), so the world
//! map scrolls smoothly while W is held: the scripts move the camera by
//! `1500 · Time.getDelta()` (times the zoom factor) on each call. Skips cleanly when
//! the Steam install isn't there.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};
use squirrel::{Number, SqTable};

/// The engine's code for W.
const KEY_W: i64 = 33;

fn camera_y(engine: &GameEngine) -> f64 {
	engine
		.run_script("::__cam_y <- ::World.getCamera().getPos().Y;", "camera")
		.expect("camera");
	SqTable::root(engine.vm()).get::<Number>("__cam_y").expect("y").0
}

#[test]
fn a_held_key_scrolls_the_world_every_frame() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::unpause_world(&engine).expect("unpause");
	for _ in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	engine.run_script("::World.getCamera().Zoom = 1.0;", "zoom").expect("zoom");
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");

	// Held for 20 frames, with the system's key repeats coming in as they do.
	engine.queue_key(KEY_W, true, 0);
	let mut steps = Vec::new();
	let mut y = camera_y(&engine);
	for frame in 0..20 {
		if frame % 3 == 0 {
			engine.queue_key(KEY_W, true, 0);
		}
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
		let now = camera_y(&engine);
		steps.push(now - y);
		y = now;
	}
	// 1500 units a second at zoom 1: 25 a frame, every frame.
	assert!(steps.iter().all(|s| (s - 25.0).abs() < 0.5), "smooth, every frame: {steps:?}");

	engine.queue_key(KEY_W, false, 0);
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	let released = camera_y(&engine);
	for _ in 0..10 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	assert_eq!(camera_y(&engine), released, "stops when released");
}
