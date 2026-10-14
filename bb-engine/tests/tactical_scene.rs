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
	// Units are never drawn on a tile the player can't see now; map objects are.
	let scene = engine.tactical_scene();
	assert!(scene.entities.iter().filter(|e| e.is_actor).all(|e| !e.visible_in_fog), "actors hide in the fog");
	assert!(scene.entities.iter().filter(|e| !e.is_actor).all(|e| e.visible_in_fog), "objects show in it");
	assert!(scene.entities.iter().any(|e| !e.is_actor), "the map has objects");
	// The player sees the tiles its units stand on, and not the whole map.
	engine
		.run_script(
			"::__ids <- \"\"; \
			 foreach (a in ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player)) ::__ids += a.getID() + \",\";",
			"player units",
		)
		.expect("player units");
	let ids = squirrel::SqTable::root(engine.vm()).get::<String>("__ids").expect("ids");
	let ids: Vec<i64> = ids.split(',').filter(|s| !s.is_empty()).map(|s| s.parse().expect("id")).collect();
	let player_tiles: Vec<usize> = scene.entities.iter().filter(|e| ids.contains(&e.id)).filter_map(|e| e.tile).collect();
	assert!(!player_tiles.is_empty(), "the player's units are on the map");
	assert!(player_tiles.iter().all(|&t| scene.tiles[t].visible), "the player sees its units' tiles");
	assert!(scene.tiles.iter().any(|t| !t.visible), "some of the map is out of sight");
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

	// Effects are game state: an icon shows for its phases, a pool stays.
	engine
		.run_script(
			"local t = ::Tactical.getTileSquare(16, 16); local S = ::Const.Tactical.Settings; \
			 ::Tactical.spawnIconEffect(\"status_effect_87\", t, S.SkillIconOffsetX, S.SkillIconOffsetY, S.SkillIconScale, \
			   S.SkillIconFadeInDuration, S.SkillIconStayDuration, S.SkillIconFadeOutDuration, S.SkillIconMovement); \
			 ::Tactical.spawnPoolEffect(::Const.BloodPoolDecals[::Const.BloodType.Red][0], t, 1.0, 1.0, 0);",
			"effects",
		)
		.expect("effects");
	let scene = engine.tactical_scene();
	assert_eq!(scene.effects.iter().filter(|e| e.brush == "status_effect_87").count(), 1, "the icon shows");
	let pools = scene.pools.len();
	assert!(pools >= 1);
	for _ in 0..90 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	let scene = engine.tactical_scene();
	assert!(scene.effects.iter().all(|e| e.brush != "status_effect_87"), "the icon is over after 1.5 s");
	assert!(scene.pools.len() >= pools, "pools stay");

	// A head flies off with the layers' sprites as the script coloured them, then lies
	// on the ground over its blood pool.
	engine
		.run_script(
			"local t = ::Tactical.getTileSquare(12, 12); \
			 ::__layer <- ::Const.BloodPoolDecals[::Const.BloodType.Red][1]; \
			 local d = ::Tactical.spawnHeadEffect(t, [::__layer], ::createVec(0, 0), -90.0, \"bust_head_dead_bloodpool\"); \
			 d[0].Color = ::createColor(\"#ff0000\"); d[0].Scale = 0.9;",
			"head",
		)
		.expect("head");
	let layer = squirrel::SqTable::root(engine.vm()).get::<String>("__layer").expect("the layer");
	let scene = engine.tactical_scene();
	let flying = scene.effects.iter().find(|e| e.brush == layer).expect("the head flies");
	assert_eq!((flying.color & 0x00ff_ffff, flying.scale), (0x00ff_0000, 0.9), "as coloured: {flying:?}");
	for _ in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	let scene = engine.tactical_scene();
	assert!(scene.effects.iter().all(|e| e.brush != layer), "landed after 0.75 s");
	assert!(scene.pools.iter().any(|p| p.brush == "bust_head_dead_bloodpool"), "its blood pool");
	assert!(scene.pools.iter().any(|p| p.brush == layer), "the head on the ground");

	// A unit's sprite: `setBrightness` changes its colour once, and `fadeOutAndHide`
	// hides it when a draw (`Tactical.render()`) finishes the fade.
	engine
		.run_script(
			&format!(
				"local s = ::Tactical.getEntityByID({unit}).getSprite(\"body\"); \
				 s.Color = ::createColor(\"#ff8020\"); s.setBrightness(0.5); s.fadeOutAndHide(200); \
				 ::__visible <- s.Visible;"
			),
			"body",
		)
		.expect("body");
	assert_eq!(squirrel::SqTable::root(engine.vm()).get::<bool>("__visible"), Some(true), "still shown while it fades");
	let body = |engine: &harness::GameEngine| {
		engine
			.tactical_scene()
			.entities
			.into_iter()
			.find(|e| e.id == unit)
			.and_then(|e| e.sprites.into_iter().find(|s| s.name == "body"))
			.expect("the body")
	};
	assert_eq!(body(&engine).color & 0x00ff_ffff, 0x007f_4010, "half as bright");
	for _ in 0..30 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	let body = body(&engine);
	assert!(!body.visible && body.color >> 24 == 0, "faded out and hidden: {body:?}");
}
