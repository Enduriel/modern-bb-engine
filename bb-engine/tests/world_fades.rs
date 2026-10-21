//! World entities fade in and out of the player's sight, as the original's do: coming
//! into sight, their sprites go clear and fade in over 300 ms of real time; going out
//! of it, they fade out over 300 ms, drawn until clear. Skips cleanly when the Steam
//! install isn't there.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};

/// The entity's alpha as drawn (its first visible sprite's), and whether it's drawn.
fn look(engine: &GameEngine, id: i64) -> (u32, bool) {
	let scene = engine.world_scene();
	let e = scene.entities.iter().find(|e| e.id == id).expect("the entity");
	let alpha = e.sprites.iter().find(|s| s.visible && !s.brush.is_empty()).map_or(0, |s| s.color >> 24);
	(alpha, e.shown)
}

#[test]
fn an_entity_fades_into_and_out_of_sight() {
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
	for _ in 0..30 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	// A party out of sight, with something to draw (a party, not a location: locations
	// stay in sight in the fog once discovered).
	let scene = engine.world_scene();
	let candidates: Vec<i64> = scene
		.entities
		.iter()
		.filter(|e| !e.shown && e.sprites.iter().any(|s| s.visible && !s.brush.is_empty()))
		.map(|e| e.id)
		.collect();
	let id = candidates
		.into_iter()
		.find(|id| {
			engine
				.run_script(&format!("::__party <- ::World.getEntityByID({id}).ClassName.find(\"party\") != null;"), "party")
				.expect("class");
			squirrel::SqTable::root(engine.vm()).get::<bool>("__party") == Some(true)
		})
		.expect("a party out of sight");
	// The fog lifts: everything is in sight.
	engine.run_script("::World.setFogOfWar(false);", "fog off").expect("fog off");

	// It comes into sight clear and fades in over 300 ms (18 frames).
	let mut alphas = Vec::new();
	for _ in 0..24 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
		let (alpha, shown) = look(&engine, id);
		assert!(shown);
		alphas.push(alpha);
	}
	assert!(alphas[0] < 40, "starts nearly clear: {alphas:?}");
	assert!(alphas.windows(2).all(|w| w[1] >= w[0]), "rises: {alphas:?}");
	assert!(alphas.iter().any(|&a| (100..160).contains(&a)), "passes half way: {alphas:?}");
	assert_eq!(*alphas.last().unwrap(), 255, "opaque after 300 ms: {alphas:?}");

	// The fog back: out of sight again, it fades out, drawn until clear.
	engine.run_script("::World.setFogOfWar(true);", "fog on").expect("fog on");
	let mut fading = Vec::new();
	for _ in 0..30 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
		let (alpha, shown) = look(&engine, id);
		if !shown {
			break;
		}
		fading.push(alpha);
	}
	assert!(fading.len() >= 10, "drawn while it fades: {fading:?}");
	assert!(fading.windows(2).all(|w| w[1] <= w[0]), "falls: {fading:?}");
	assert!(!look(&engine, id).1, "not drawn once clear");
}
