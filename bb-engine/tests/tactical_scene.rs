//! A battle's map as a renderer reads it (`Engine::tactical_scene`): every tile has
//! terrain, many carry details, the terrain brushes' transitions are registered, and
//! the camera looks at the map. Skips cleanly when the Steam install isn't there.

use std::path::Path;

use bb_engine::harness;

#[test]
fn a_battle_map_is_ready_to_draw() {
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
	for frame in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
	}
	harness::engage_nearest_hostile(&engine).expect("engage");
	// Until the battle's map is built (the tactical state makes it a few frames in).
	let mut scene = engine.tactical_scene();
	for frame in 0..3_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
		scene = engine.tactical_scene();
		if !scene.tiles.is_empty() && scene.tiles.iter().all(|t| !t.brush.is_empty()) {
			break;
		}
	}
	assert_eq!(scene.tiles.len() as i64, scene.width * scene.height);
	assert!(scene.width > 0 && scene.height > 0);
	let with_terrain = scene.tiles.iter().filter(|t| !t.brush.is_empty()).count();
	assert_eq!(with_terrain, scene.tiles.len(), "every tile has a terrain brush");
	let details: usize = scene.tiles.iter().map(|t| t.details.len()).sum();
	assert!(details > 100, "the map is dressed ({details} details)");
	// Every terrain brush on the map has its transitions registered.
	for tile in &scene.tiles {
		assert!(scene.transitions.contains_key(&tile.brush), "transitions for {}", tile.brush);
	}
	assert!(scene.tiles.iter().any(|t| t.level > 0), "the map has height");
	assert!(scene.camera.zoom > 0.0);
	// The battle's light, as `setupAmbience` sets it for the time of day.
	engine
		.run_script(
			"local t = ::World.getTime().TimeOfDay; \
			 ::__light <- ::Const.Tactical.AmbientLightingColor.Time[t]; \
			 ::__saturation <- ::Const.Tactical.AmbientLightingSaturation.Time[t];",
			"light",
		)
		.expect("light");
	let root = squirrel::SqTable::root(engine.vm());
	let hex = root.get::<String>("__light").expect("colour");
	let color = 0xff00_0000 | u32::from_str_radix(&hex[1..], 16).expect("hex");
	assert_eq!(scene.light.color, color, "ambient colour {hex}");
	assert_eq!(scene.light.saturation, root.get::<f64>("__saturation").expect("saturation") as f32);

	// A unit's status effects show as icons over its bars (`setOverlayIcons`).
	let mut unit = None;
	for frame in 0..3_000 {
		unit = engine.tactical_scene().entities.iter().find(|e| e.is_actor && e.overlay.is_some()).map(|e| e.id);
		if unit.is_some() {
			break;
		}
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
	}
	let unit = unit.expect("a unit with bars");
	engine
		.run_script(
			&format!(
				"local a = ::Tactical.getEntityByID({unit}); \
				 a.getSkills().add(::new(\"scripts/skills/effects/stunned_effect\")); \
				 a.updateOverlay();"
			),
			"stun",
		)
		.expect("stun");
	let icons = engine
		.tactical_scene()
		.entities
		.into_iter()
		.find(|e| e.id == unit)
		.and_then(|e| e.overlay)
		.map(|o| o.icons)
		.expect("the unit's bars");
	assert!(icons.iter().any(|i| i == "status_effect_05_mini"), "the stun's icon: {icons:?}");
}
