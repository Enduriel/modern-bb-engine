//! A battle drawn in steps (`squirrel_bb::scene::steps`), as `bb` draws it: on the
//! player's turn, nobody acting, in a storm, at 180 frames a second. The active unit hops,
//! the details turn, the rain falls and fades; the scene shows them as they are at each
//! step of 1/60 s and holds them in between, so two frames in three show what the last
//! did. The game's state still moves every frame. Paused, the rain's fades step the same
//! way; without steps, every frame is the battle as it is.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};
use squirrel_bb::scene::tactical::TacticalScene;

const STORM: &str = r#"
	local w = ::Tactical.getWeather(), rain = w.createRainSettings();
	rain.MinDrops = 150;
	rain.MaxDrops = 150;
	rain.NumSplats = 50;
	rain.MinVelocity = 400.0;
	rain.MaxVelocity = 500.0;
	w.buildRain(rain);
"#;

const FRAME_MS: f64 = 1000.0 / 180.0;

/// A frame without the scripted player: the engine's and the UI client's.
fn frame(engine: &harness::GameEngine, ms: f64) {
	engine.frame(ms).expect("frame");
	engine.pump_ui().expect("ui");
}

/// The battle `bb-render`'s screenshots start (a bandit fight on `tactical.hills`), on
/// the player's first turn; `None` without the Steam install.
fn battle() -> Option<harness::GameEngine> {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return None;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	for _ in 0..40 {
		frame(&engine, 1000.0 / 60.0);
	}
	engine
		.run_script(
			include_str!("../../bb-render/examples/battle_setup.nut"),
			"setup",
		)
		.expect("setup");
	let piloted = |engine: &harness::GameEngine| {
		frame(engine, 1000.0 / 60.0);
		engine.run_script("::__pilot();", "pilot").expect("pilot");
	};
	for _ in 0..120 {
		piloted(&engine);
	}
	engine
		.run_script("::__startBattle();", "battle")
		.expect("battle");
	let players_turn = |engine: &harness::GameEngine| {
		player::eval_i64(engine, "::__playerTurn() ? 1 : 0").unwrap() == Some(1)
	};
	for _ in 0..6000 {
		piloted(&engine);
		if players_turn(&engine) {
			break;
		}
	}
	assert!(players_turn(&engine), "the player's turn");
	engine
		.run_script("::__battleCamera(); ::__fogOff();", "camera")
		.expect("camera");
	engine.run_script(STORM, "storm").expect("build the rain");
	Some(engine)
}

/// `frames` frames of [`FRAME_MS`]: each scene against the battle as it is at a step and
/// against the last scene between steps. Returns the steps taken, the frames that showed
/// what the last did, and the frames whose battle as it is changed.
fn run(engine: &harness::GameEngine, frames: usize) -> (usize, usize, usize) {
	let (mut last, mut last_live) = (engine.tactical_scene(), engine.tactical_scene_as_is());
	let mut taken = engine.scene_steps_taken();
	let (mut steps, mut still, mut moved) = (0, 0, 0);
	for n in 0..frames {
		frame(engine, FRAME_MS);
		let scene = engine.tactical_scene();
		let live = engine.tactical_scene_as_is();
		let now = engine.scene_steps_taken();
		if now != taken {
			steps += 1;
			assert!(
				scene == live,
				"frame {n}: at a step, the battle as it is: {} differ",
				differing(&scene, &live)
			);
		} else {
			assert!(
				scene == last,
				"frame {n}: between steps, the scene held: {} differ",
				differing(&scene, &last)
			);
		}
		still += usize::from(scene == last);
		moved += usize::from(live != last_live);
		(last, last_live, taken) = (scene, live, now);
	}
	(steps, still, moved)
}

/// The scenes' fields that differ, for a failure.
fn differing(a: &TacticalScene, b: &TacticalScene) -> String {
	let mut out = Vec::new();
	let mut field = |name: &str, same: bool| {
		if !same {
			out.push(name.to_string());
		}
	};
	field("camera", a.camera == b.camera);
	field("orientation", a.orientation == b.orientation);
	field("rain", a.rain == b.rain);
	field("rain splats", a.rain_splats == b.rain_splats);
	field("clouds", a.clouds == b.clouds);
	field("particles", a.particles == b.particles);
	field("effects", a.effects == b.effects);
	field("overlay icons", a.overlay_icons == b.overlay_icons);
	field("pools", a.pools == b.pools);
	field("tiles", a.tiles == b.tiles);
	for (x, y) in a.entities.iter().zip(&b.entities) {
		if x != y {
			let what = if x.pos != y.pos {
				"pos"
			} else if x.overlay != y.overlay {
				"overlay"
			} else {
				"sprites"
			};
			out.push(format!("entity {} {what}", x.id));
		}
	}
	if a.entities.len() != b.entities.len() {
		out.push("entity count".into());
	}
	out.join(", ")
}

#[test]
fn a_battle_in_steps_shows_its_animations_at_each_60th_of_a_second() {
	let Some(engine) = battle() else { return };
	engine.set_scene_steps(Some(1.0));
	// The frames' mean length settles at 1/180 s.
	for _ in 0..60 {
		frame(&engine, FRAME_MS);
		engine.tactical_scene();
	}
	// Two seconds: 120 ticks of 1/60 s, give or take the frames' rounding.
	let (steps, still, moved) = run(&engine, 360);
	assert!((115..=125).contains(&steps), "{steps} steps in 2 s");
	assert!(still >= 230, "{still} of 360 frames show what the last did");
	assert!(
		moved >= 355,
		"the battle as it is moves every frame ({moved} of 360)"
	);
	assert_eq!(
		engine.scene_step_doublings(),
		Some(0),
		"the hop and the rain take 1/60 s"
	);

	// Paused: the hop and the details stop, the rain's fades go on in steps.
	engine
		.run_script("::Tactical.State.setPause(true);", "pause")
		.unwrap();
	let (steps, still, _) = run(&engine, 360);
	assert!((115..=125).contains(&steps), "{steps} steps in 2 s, paused");
	assert!(
		still >= 230,
		"{still} of 360 frames show what the last did, paused"
	);

	// Without the rain nothing moves: the battle is still, its step the longest.
	engine
		.run_script("::Tactical.getWeather().clear();", "clear")
		.unwrap();
	for _ in 0..3 {
		frame(&engine, FRAME_MS);
		engine.tactical_scene();
	}
	let (_, still, _) = run(&engine, 180);
	assert_eq!(still, 180, "a paused battle without weather is still");
	assert_eq!(
		engine.scene_step_doublings(),
		Some(squirrel_bb::scene::steps::MOST_DOUBLINGS)
	);

	// Without steps, every frame is the battle as it is.
	engine
		.run_script("::Tactical.State.setPause(false);", "unpause")
		.unwrap();
	engine.set_scene_steps(None);
	let (steps, still, _) = run(&engine, 60);
	assert_eq!(steps, 60, "a step every frame");
	assert_eq!(still, 0, "the hop moves every frame");
}
