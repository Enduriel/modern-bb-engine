//! A battle's rain, as the battle's scene exports it: a storm built as the battle's
//! weather builds one starts with three quarters of its drops (scaled by the screen's
//! width over 1024) and fills up on the next update; the drops fade in and fall along
//! the rain's direction; splats come and go on the ground; `IsRaining` holds while it
//! rains, and `weather.clear()` stops it.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

const STORM: &str = r#"
	local w = ::Tactical.getWeather(), rain = w.createRainSettings();
	rain.MinDrops = 150;
	rain.MaxDrops = 150;
	rain.NumSplats = 50;
	rain.MinVelocity = 400.0;
	rain.MaxVelocity = 500.0;
	rain.MinAlpha = 1.0;
	rain.MaxAlpha = 1.0;
	rain.MinScale = 0.75;
	rain.MaxScale = 1.0;
	w.buildRain(rain);
"#;

#[test]
fn a_battle_storm_falls_and_stops() {
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
	for _ in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	harness::engage_nearest_hostile(&engine).expect("engage");
	let in_battle = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible() && ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player).len() > 0";
	for _ in 0..3_000 {
		if player::eval_i64(&engine, &format!("({in_battle}) ? 1 : 0")).unwrap() == Some(1) {
			break;
		}
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	let raining = |engine: &harness::GameEngine| player::eval_i64(engine, "::Tactical.getWeather().IsRaining ? 1 : 0").unwrap() == Some(1);
	let width = player::eval_i64(&engine, "::Settings.getVideoMode().Width").unwrap().unwrap() as f32;
	let factor = width / 1024.0;

	engine.run_script(STORM, "storm").expect("build the rain");
	assert!(raining(&engine), "IsRaining");
	let scene = engine.tactical_scene();
	assert_eq!(scene.rain.len(), (150.0 * factor * 0.75) as usize, "three quarters of the drops to start");
	assert_eq!(scene.rain_splats.len(), (50.0 * factor * 0.75) as usize, "and of the splats");
	assert!(scene.rain.iter().all(|d| d.brush == "rain_01" && d.color >> 24 < 0x40), "fading in from clear");
	let before: Vec<(f32, f32)> = scene.rain.iter().map(|d| d.pos).collect();

	harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	let scene = engine.tactical_scene();
	assert_eq!(scene.rain.len(), (150.0 * factor).round() as usize, "all of them after an update");
	// The first drops fall down and a little left (direction (-0.3, -0.7), y up); a
	// slanted one (-0.36, -0.64).
	for (d, p) in scene.rain.iter().zip(&before) {
		let (dx, dy) = (d.pos.0 - p.0, d.pos.1 - p.1);
		let r = dx / dy;
		assert!(dy < 0.0 && ((r - 0.3 / 0.7).abs() < 0.01 || (r - 0.36 / 0.64).abs() < 0.01), "falling: ({dx}, {dy})");
	}
	for _ in 0..30 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	let scene = engine.tactical_scene();
	assert!(scene.rain.iter().any(|d| d.color >> 24 == 0xff), "faded in");
	assert!(!scene.rain_splats.is_empty(), "splats");

	engine.run_script("::Tactical.getWeather().clear();", "clear").unwrap();
	assert!(!raining(&engine), "stopped");
	let scene = engine.tactical_scene();
	assert!(scene.rain.is_empty() && scene.rain_splats.is_empty());
}
