//! Sprites play their brushes' frames on virtual time (see `sprite_anim`): a world
//! party's fight sprite (`battle_animation`, nine frames of 100 ms, looping) cycles
//! while the world runs and it shows, and holds while it's paused or hidden (the
//! original steps only a sprite it draws: visible, with a brush). Skips cleanly when
//! the Steam install isn't there.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};

/// Every party's `combat` sprite frame, by entity id.
fn combat_frames(engine: &GameEngine) -> Vec<(i64, u32)> {
	engine
		.world_scene()
		.entities
		.iter()
		.filter_map(|e| e.sprites.iter().find(|s| *s.name == *"combat" && *s.brush == *"battle_animation").map(|s| (e.id, s.frame)))
		.collect()
}

#[test]
fn a_fight_sprite_cycles_on_virtual_time() {
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
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	let (id, _) = *combat_frames(&engine).first().expect("a party with a fight sprite");
	let frame_of = |engine: &GameEngine| combat_frames(engine).into_iter().find(|&(e, _)| e == id).map(|(_, f)| f);
	let show = |on: bool| {
		let src = format!("::World.getEntityByID({id}).getSprite(\"combat\").Visible = {on};");
		engine.run_script(&src, "show the fight sprite").expect("show");
	};

	// Hidden (as a party not fighting has it): it holds.
	show(false);
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	let hidden = frame_of(&engine);
	for _ in 0..30 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
		assert_eq!(frame_of(&engine), hidden, "a hidden sprite isn't stepped");
	}
	show(true);

	// Running: a second of frames shows every one of the nine, in order.
	let mut seen = Vec::new();
	for _ in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
		seen.push(frame_of(&engine).expect("the party"));
	}
	let mut shown: Vec<u32> = seen.clone();
	shown.dedup();
	assert!(shown.len() >= 9, "steps through its frames: {seen:?}");
	assert!(seen.iter().all(|&f| f < 9));
	assert!(
		shown.windows(2).all(|w| w[1] == (w[0] + 1) % 9),
		"one frame at a time, wrapping: {shown:?}"
	);

	// Paused: it holds.
	engine.run_script("::__bootWorldState.setPause(true);", "pause").expect("pause");
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	let held = frame_of(&engine);
	for _ in 0..30 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
		assert_eq!(frame_of(&engine), held, "a paused world holds it");
	}
}
