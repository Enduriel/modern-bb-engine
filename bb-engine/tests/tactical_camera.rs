//! The battle camera as the windowed game runs it (`BattleCamera::Glides`): a move to
//! a tile takes frames, says it's moving until it arrives, and arrives centred, while
//! the battle carries on. Skips cleanly when the Steam install isn't there.

use std::path::Path;

use bb_engine::harness;
use squirrel::{SqArray, SqTable};

#[test]
fn a_gliding_battle_camera_moves_over_frames() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install_with(dir, |c| c.battle_camera = squirrel_bb::BattleCamera::Glides).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::unpause_world(&engine).expect("unpause");
	for frame in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
	}
	harness::engage_nearest_hostile(&engine).expect("engage");
	for frame in 0..3_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
		let scene = engine.tactical_scene();
		if !scene.tiles.is_empty() && scene.tiles.iter().all(|t| !t.brush.is_empty()) {
			break;
		}
	}

	let camera = |code: &str| -> Vec<String> {
		engine
			.run_script(
				&format!(
					"local c = ::Tactical.getCamera(); local far = ::Tactical.getTileSquare(9, 8); {code}; \
					 local p = c.getPos(); ::__cam <- [c.isMoving(), c.isCenteredOnTile(far), c.isMovingToTile(far), p.X, p.Y];"
				),
				"camera",
			)
			.expect("camera");
		let a = SqTable::root(engine.vm()).get::<SqArray>("__cam").expect("answers");
		(0..a.len()).map(|i| a.get::<squirrel::Number>(i).map_or_else(|| format!("{:?}", a.get::<bool>(i)), |n| n.0.to_string())).collect()
	};
	// The battle paused, so no unit's turn takes the camera elsewhere meanwhile (the
	// camera moves on real time).
	engine.run_script("::Tactical.State.setPause(true);", "pause").expect("pause");
	let start = camera("c.setPos(::Tactical.getTileSquare(22, 18).Pos); c.moveToTile(far)");
	assert_eq!(&start[..3], ["Some(true)", "Some(false)", "Some(true)"], "under way: {start:?}");
	let mut frames = 0;
	loop {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
		frames += 1;
		let now = camera("");
		if now[0] == "Some(false)" {
			assert_eq!(now[1], "Some(true)", "arrived centred: {now:?}");
			break;
		}
		assert!(frames < 600, "still moving after {frames} frames: {now:?}");
	}
	assert!(frames > 5, "a glide takes frames ({frames})");
}
