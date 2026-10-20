//! The frame's sprite passes visit only the owners noted as having sprites that may
//! change on their own (see `squirrel_bb`'s `moving_sprites`). Every frame of a running
//! world and of a whole battle fought by the AI, no sprite that may change (a torque,
//! frames to play, a fade) belongs to an owner that isn't noted, so the passes miss
//! nothing. Skips cleanly when the Steam install isn't there.

use std::path::Path;

use bb_engine::harness;

const IN_BATTLE: &str = "((\"State\" in ::Tactical) && ::Tactical.State != null) ? 1 : 0";

#[test]
fn no_moving_sprite_is_left_unnoted() {
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
	let check = |when: &str| assert_eq!(engine.unnoted_moving_sprites(), 0, "{when}: moving sprites the passes would miss");

	for i in 0..600 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
		check(&format!("world frame {i}"));
	}

	engine
		.run_script("::__autoplayFight = true; ::__autoplayDieAsPlayers = true;", "fight")
		.expect("fight");
	harness::engage_nearest_hostile(&engine).expect("engage");
	let in_battle = || bb_engine::tactical_player::eval_i64(&engine, IN_BATTLE).ok().flatten() == Some(1);
	let (mut battle_frames, mut back) = (0, 0);
	for i in 0..20_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
		check(&format!("frame {i} after the engage"));
		if in_battle() {
			battle_frames += 1;
		} else if battle_frames > 0 {
			// Back on the world map for a while.
			back += 1;
			if back == 120 {
				break;
			}
		}
	}
	assert!(battle_frames > 600, "fought a battle ({battle_frames} frames)");
	assert_eq!(back, 120, "the battle ended and the world ran again");
}
