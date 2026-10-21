//! A party's movement on the world map is smooth: the player's party, sent along a path
//! of several tiles, moves a step of about its speed × the frame's time every frame,
//! frames of any length, with no jump at the tiles' edges or waypoints, and with the
//! camera locked on it the party keeps its place on the screen.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

/// The player's position, then the camera's.
const SAMPLE: &str = r#"(function() {
	local p = ::World.State.getPlayer().getPos(), c = ::World.getCamera().getPos();
	return p.X + " " + p.Y + " " + c.X + " " + c.Y + " " + ::Time.getVirtualTimeF();
})()"#;

#[test]
fn the_players_party_moves_smoothly_along_a_path() {
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
	// The player's party starts out of range of the player's view (which its first
	// update sets), so its first step covers that first interval, as the original's
	// out-of-range rule has it; the walk starts after that.
	for frame in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
	}
	// A path to a reachable tile a few tiles away, as a click on the map sets one.
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
			::__walkPath <- best.getSize();
			player.setPath(best);
			::Settings.getTempGameplaySettings().CameraLocked = true;
			"#,
			"path",
		)
		.expect("set a path");
	let waypoints = player::eval_i64(&engine, "::__walkPath").unwrap().unwrap();
	assert!(waypoints >= 3, "a path of a few tiles ({waypoints})");

	// Frames of varying length, as a real frame clock gives them (4 to 33 ms).
	let mut samples: Vec<Vec<f64>> = Vec::new();
	for frame in 0..1500 {
		let ms = [16.7, 4.2, 33.3, 6.9, 12.0, 25.0][frame % 6];
		harness::run_frame(&engine, ms).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
		let s = player::eval_string(&engine, SAMPLE).unwrap().unwrap();
		samples.push(s.split(' ').map(|x| x.parse().unwrap()).collect());
	}
	// The party's speed each frame: its step over the virtual time that passed.
	let speeds: Vec<(usize, f64, f64)> = samples
		.windows(2)
		.enumerate()
		.filter_map(|(i, w)| {
			let step = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
			let dt = w[1][4] - w[0][4];
			(step > 0.0 && dt > 0.0).then_some((i, step, step / dt))
		})
		.collect();
	assert!(speeds.len() > 100, "the party moved ({} frames)", speeds.len());
	let mut sorted: Vec<f64> = speeds.iter().map(|s| s.2).collect();
	sorted.sort_by(f64::total_cmp);
	let median = sorted[sorted.len() / 2];
	let worst = speeds.iter().copied().fold((0, 0.0, 0.0), |a, s| if s.2 > a.2 { s } else { a });
	eprintln!("median speed {median:.1}/s, fastest {:.1}/s (a step of {:.2}) at frame {}", worst.2, worst.1, worst.0);
	for (i, step, speed) in &speeds {
		if *speed > median * 2.0 {
			eprintln!("frame {i}: step {step:.2} at {speed:.1}/s ({:?} -> {:?})", samples[*i], samples[*i + 1]);
		}
	}
	// The speed varies with the terrain (roads, rivers, forests), not by more.
	assert!(worst.2 <= median * 3.0, "the party jumped {:.2} in one frame ({:.1}/s, median {median:.1}/s)", worst.1, worst.2);
	// With the camera locked on the party, its place on the screen moves by no more than
	// the camera's whole-unit rounding and a frame's step (the camera takes the party's
	// position before the party moves that frame).
	for (i, w) in samples.windows(2).enumerate().skip(1) {
		let on_screen = |s: &[f64]| (s[0] - s[2], s[1] - s[3]);
		let (a, b) = (on_screen(&w[0]), on_screen(&w[1]));
		let drift = (b.0 - a.0).hypot(b.1 - a.1);
		assert!(drift < 4.0, "frame {i}: the party moved {drift:.2} on the screen ({:?} -> {:?})", w[0], w[1]);
	}
}
