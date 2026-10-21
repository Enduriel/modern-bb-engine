//! A new campaign's world map as a renderer reads it (`Engine::world_scene`): every
//! land tile has terrain, decorations sit on their tiles, the player's party has its
//! sprites in the order the scripts added them, the fog of war lies where the
//! original's does, and birds fly over it. Skips cleanly when the Steam install isn't there.

use std::path::Path;

use bb_engine::harness::{self, STEAM_DATA_DIR};
use squirrel::{Number, SqStrongRef, SqTable};

#[test]
fn a_new_campaign_world_is_ready_to_draw() {
	let dir = Path::new(STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{STEAM_DATA_DIR} not present"));
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare");
	harness::start_new_campaign(&engine).expect("start");
	let scene = engine.world_scene();

	assert_eq!(scene.tiles.len(), 140 * 170);
	// The world state's 12 birds, flying west over the map.
	assert_eq!(scene.birds.len(), 12, "birds");
	assert!(scene.birds.iter().all(|b| b.brush.starts_with("world_birds_0") && (45.0..=65.0).contains(&b.velocity)));
	// Ocean and shore tiles have no terrain brush: the water pass draws them.
	const OCEAN: i64 = 1;
	const SHORE: i64 = 16;
	let (water, land): (Vec<_>, Vec<_>) = scene.tiles.iter().partition(|t| t.terrain == OCEAN || t.terrain == SHORE);
	assert!(land.iter().all(|t| !t.brush.is_empty()), "every land tile has terrain");
	assert!(water.iter().all(|t| t.brush.is_empty()));
	assert!(!water.is_empty());
	let details: usize = scene.tiles.iter().map(|t| t.details.len()).sum();
	assert!(details > 10_000, "worldgen decorates the map ({details} details)");
	let houses = scene.tiles.iter().flat_map(|t| &t.details).filter(|d| d.brush.starts_with("world_houses_"));
	assert!(houses.clone().count() > 0);
	// Scripts compute in f32, so the 0.85 they set is the f32 nearest it.
	assert!(houses.clone().all(|d| d.scale == f64::from(0.85f32)), "settlements shrink their houses");

	engine.run_script("::__playerId <- ::World.State.getPlayer().getID();", "player").expect("player");
	let id = SqTable::root(engine.vm()).get::<SqStrongRef>("__playerId").expect("id");
	let id = id.get_value::<Number>(engine.vm()).expect("number").0 as i64;
	let player = scene.entities.iter().find(|e| e.id == id).expect("player in the scene");
	assert_eq!(player.pos, (14160.0, 16740.0));
	let names: Vec<&str> = player.sprites.iter().map(|s| &*s.name).collect();
	let order = ["banner", "base", "body", "bodyUp", "combat", "lighting", "zoom_banner"];
	let positions: Vec<usize> = order.iter().map(|n| names.iter().position(|m| m == n).expect(n)).collect();
	assert!(positions.is_sorted(), "added in script order: {names:?}");
	let body = player.sprites.iter().find(|s| *s.name == *"body").expect("body");
	assert_eq!(&*body.brush, "figure_player_01");
	let selection = player.sprites.iter().find(|s| *s.name == *"selection").expect("selection");
	assert_eq!(selection.offset, (-30.0, 30.0));
	assert!(!selection.scaling);

	// Fog of war: the start uncovers the same tiles as the original's (13,560 for this
	// seed, probed 2026-10-14). The mask spans the map to the tile past its last corner,
	// is clear around the player and still fogged where nothing was uncovered.
	assert!(scene.fog_of_war);
	assert_eq!(scene.tiles.iter().filter(|t| t.discovered).count(), 13_560);
	assert_eq!(scene.extent, (25320.0, 20460.0));
	let mask = engine.world_grid().take_fog_patch().expect("the new map's mask");
	assert_eq!((mask.x, mask.y, mask.width, mask.height), (0, 0, 2048, 2048));
	let fog_at = |x: f32, y: f32| {
		let (i, j) = ((x / 25320.0 * 2048.0) as usize, (y / 20460.0 * 2048.0) as usize);
		mask.alpha[j * 2048 + i]
	};
	assert_eq!(fog_at(14160.0, 16740.0), 0, "clear around the player");
	assert_eq!(fog_at(16600.0, 18150.0), 255, "fogged off to the north-east");
	assert!(engine.world_grid().take_fog_patch().is_none(), "unchanged since");

	// Region names come from worldgen's seeded stream: the original's screenshot of
	// this seed shows these three (2026-10-13).
	let names: Vec<String> = scene.region_texts.iter().map(|t| String::from_utf8_lossy(&t.text).into_owned()).collect();
	for name in ["Clan's Claim", "Dreaded Mire", "Bleakwolds"] {
		assert!(names.iter().any(|n| n == name), "{name} in {names:?}");
	}
	assert!(scene.region_texts.iter().all(|t| [0.5, 0.75, 1.0].contains(&t.scale)));

	// A label keeps whatever bytes the scripts give it, UTF-8 or not: the original draws
	// one glyph per byte (Windows-1252's é is 0xE9).
	engine
		.run_script(
			r#"
			local label = ::World.EntityManager.getSettlements()[0].getLabel("name");
			label.Text = "A" + (0xE9).tochar();
			::__labelLength <- label.Text.len();
			"#,
			"label bytes",
		)
		.expect("label bytes");
	assert_eq!(SqTable::root(engine.vm()).get::<i64>("__labelLength"), Some(2));
	assert!(
		engine.world_scene().entities.iter().flat_map(|e| &e.labels).any(|l| *l.text == [0x41, 0xE9]),
		"the scene has the label's bytes"
	);

	// Footprints and waves, as scripts make and query them.
	engine
		.run_script(
			r#"
			local p = ::World.State.getPlayer().getPos();
			// The view the player's party sets each update (none has run yet).
			::World.setPlayerPos(p);
			::World.setPlayerVisionRadius(500.0);
			local at = ::createVec(p.X + 40, p.Y);
			::World.spawnFootprint(at, "world_footprints_human_01_01", 1.0, 30.0, 1.0, 2);
			local seen = ::World.getAllFootprintsAtPos(at, ::Const.World.FootprintsType.COUNT);
			::__tracks <- [seen.len(), seen[2], seen[1], ::World.getDirection8FromTo(p, at), ::World.getDirection8FromTo(at, p)];
			::World.spawnWaveSprite("waves_01", p, ::createVec(-15, -15), 1.25);
			::World.spawnWaveSprite("no_such_brush", p, ::createVec(-15, -15), 1.25);
			"#,
			"tracks",
		)
		.expect("footprints");
	let tracks = SqTable::root(engine.vm()).get::<squirrel::SqArray>("__tracks").expect("tracks");
	let count = tracks.get::<Number>(0).expect("count").0 as usize;
	assert!(count > 2);
	assert_eq!((tracks.get::<bool>(1), tracks.get::<bool>(2)), (Some(true), Some(false)));
	// East is 2 and west 6 of the 8 directions.
	assert_eq!((tracks.get::<Number>(3).map(|n| n.0), tracks.get::<Number>(4).map(|n| n.0)), (Some(2.0), Some(6.0)));
	let scene = engine.world_scene();
	assert_eq!(scene.footprints.len(), 1);
	assert_eq!(scene.waves.len(), 1, "an unknown brush makes no wave");

	// Worldgen's brushes queued their atlases for loading.
	let hints = engine.sprites().take_hints();
	for atlas in ["gfx/world_tiles.png", "gfx/world_detail.png", "gfx/world_entity_0.png", "gfx/world_settlement.png"] {
		assert!(hints.iter().any(|h| h == atlas), "{atlas} hinted: {hints:?}");
	}

	// A sprite slot answers like the original's (probed 2026-10-13).
	engine
		.run_script(
			r#"
			local party = ::World.State.getPlayer();
			local s = party.addSprite("probe");
			local r = [s.HasBrush, s.getBrush() == null];
			s.setBrush("no_such_brush");
			r.extend([s.HasBrush, s.getBrush() == null]);
			s.setBrush("world_birds_01");
			r.extend([s.HasBrush, s.getBrush().Name == "world_birds_01", s.getBrush().TextureName == "gfx/world_weather.png"]);
			s.setBrush("no_such_brush");
			r.push(s.HasBrush);
			s.setBrush("world_birds_01");
			s.setBrush("");
			r.push(s.HasBrush);
			s.setBrush("world_birds_01");
			s.resetBrush();
			r.push(s.HasBrush);
			r.push(party.getSprite("body").getBrush().TextureName == "gfx/world_entity_0.png");
			::__slot <- r;
			"#,
			"slot",
		)
		.expect("slot");
	let answers = SqTable::root(engine.vm()).get::<squirrel::SqArray>("__slot").expect("answers");
	let answers: Vec<bool> = (0..answers.len()).map(|i| answers.get::<bool>(i).expect("bool")).collect();
	assert_eq!(answers, [false, true, false, true, true, true, true, false, false, false, true]);

	// Sprite fades: Alpha is an integer; a fade shows when drawn and only a draw at its
	// end finishes it (see SpriteColor).
	engine
		.run_script(
			r#"
			local s = ::World.State.getPlayer().addSprite("fade_probe");
			s.Alpha = 0;
			local r = [s.Alpha, s.IsFadingDone];
			s.fadeToAlpha(128, 0);
			r.extend([s.Alpha, s.IsFadingDone]);
			s.fadeIn(100);
			r.extend([s.Alpha, s.IsFadingDone]);
			::__fade <- r;
			"#,
			"fade",
		)
		.expect("fade");
	let fade = SqTable::root(engine.vm()).get::<squirrel::SqArray>("__fade").expect("fade");
	let n = |i| fade.get::<Number>(i).map(|n| n.0);
	assert_eq!((n(0), fade.get::<bool>(1)), (Some(0.0), Some(true)));
	assert_eq!((n(2), fade.get::<bool>(3)), (Some(128.0), Some(true)), "a zero-length fade is immediate");
	assert_eq!((n(4), fade.get::<bool>(5)), (Some(128.0), Some(false)), "fading, the start alpha reads back");
	for _ in 0..20 {
		engine.frame(16.0).expect("frame");
	}
	engine
		.run_script(
			r#"local s = ::World.State.getPlayer().getSprite("fade_probe"); ::__fade <- [s.Alpha, s.IsFadingDone];"#,
			"fade after",
		)
		.expect("fade after");
	let fade = SqTable::root(engine.vm()).get::<squirrel::SqArray>("__fade").expect("fade");
	assert_eq!((fade.get::<Number>(0).map(|n| n.0), fade.get::<bool>(1)), (Some(255.0), Some(true)), "drawn past its end");

	// Sprites turn by their torque as virtual time passes (see `spin`): a brush brings
	// its rotation speed (the selection ring's 45), and scripts can set one (the
	// swamp's fog). The first frame only notes the time.
	engine
		.run_script(
			r#"
			::World.State.setAutoPause(false);
			::World.State.setPause(false);
			local p = ::World.State.getPlayer();
			local ring = p.addSprite("spin_probe");
			ring.setBrush("world_party_selection");
			local fog = p.getTile().spawnDetail("world_swamp_fog_05", ::Const.World.ZLevel.Object + 305, 0);
			fog.Scale = 0.777;
			fog.Torque = -15;
			::__spin <- [ring.Torque, typeof fog.Torque, fog.Torque];
			"#,
			"spin",
		)
		.expect("spin");
	let spin = SqTable::root(engine.vm()).get::<squirrel::SqArray>("__spin").expect("spin");
	assert_eq!(spin.get::<Number>(0).map(|n| n.0), Some(45.0));
	assert_eq!(spin.get::<String>(1).as_deref(), Some("float"), "Torque is a float");
	assert_eq!(spin.get::<Number>(2).map(|n| n.0), Some(-15.0));
	let turned = |engine: &harness::GameEngine| {
		engine
			.run_script(
				r#"::__turned <- [::Time.getVirtualTimeF(), ::World.State.getPlayer().getSprite("spin_probe").Rotation];"#,
				"turned",
			)
			.expect("turned");
		let t = SqTable::root(engine.vm()).get::<squirrel::SqArray>("__turned").expect("turned");
		let fog = engine
			.world_scene()
			.tiles
			.iter()
			.flat_map(|t| t.details.clone())
			.find(|d| (d.scale - 0.777).abs() < 1e-6)
			.unwrap_or_else(|| {
				let fogs: Vec<_> = engine
					.world_scene()
					.tiles
					.iter()
					.flat_map(|t| t.details.clone())
					.filter(|d| &*d.brush == "world_swamp_fog_05")
					.map(|d| (d.scale, d.rotation))
					.collect();
				panic!("the fog: {fogs:?}")
			});
		(t.get::<Number>(0).unwrap().0, t.get::<Number>(1).unwrap().0, fog.rotation)
	};
	engine.frame(16.0).expect("frame");
	let (t0, ring0, fog0) = turned(&engine);
	for _ in 0..20 {
		engine.frame(16.0).expect("frame");
	}
	let (t1, ring1, fog1) = turned(&engine);
	assert!(t1 > t0, "virtual time runs ({t0} -> {t1})");
	let close = |a: f64, b: f64| (a - b).abs() < 0.01;
	assert!(close(ring1 - ring0, 45.0 * (t1 - t0)), "the ring turned {} in {} s", ring1 - ring0, t1 - t0);
	assert!(close(fog1 - fog0, -15.0 * (t1 - t0)), "the fog turned {} in {} s", fog1 - fog0, t1 - t0);
	engine.run_script("::World.State.setPause(true);", "pause").expect("pause");
	engine.frame(16.0).expect("frame");
	let (_, ring2, _) = turned(&engine);
	for _ in 0..5 {
		engine.frame(16.0).expect("frame");
	}
	let (_, ring3, _) = turned(&engine);
	assert_eq!(ring2, ring3, "paused, nothing turns");

	// The camera (see `camera_motion`): the map's bounds keep the view on it, moves
	// glide and zooms ease with the frames.
	let camera = |engine: &harness::GameEngine, code: &str| {
		engine
			.run_script(
				&format!("local cam = ::World.getCamera(); {code}; local p = cam.getPos(); ::__cam <- [p.X, p.Y, cam.Zoom];"),
				"camera",
			)
			.expect("camera");
		let c = SqTable::root(engine.vm()).get::<squirrel::SqArray>("__cam").expect("camera");
		(0..3).map(|i| c.get::<Number>(i).unwrap().0).collect::<Vec<f64>>()
	};
	let (w, h, _) = engine.video().screen();
	let corner = camera(&engine, "cam.Zoom = 1.0; cam.setPos(::createVec(-5000, -5000))");
	assert_eq!(corner, [120.0 + (w / 2) as f64, 60.0 + (h / 2) as f64, 1.0], "pushed back onto the map");
	let start = camera(&engine, "cam.setPos(::createVec(5000, 5000)); cam.moveToPos(6000, 5000); cam.zoomTo(2.0, 4.0)");
	assert_eq!(start, [5000.0, 5000.0, 1.0], "a glide and a zoom start on the next frame");
	engine.frame(16.0).expect("frame");
	let step = camera(&engine, "");
	assert!(step[0] > 5000.0 && step[0] < 6000.0 && step[1] == 5000.0, "gliding: {step:?}");
	assert!((step[2] - 1.064).abs() < 1e-4, "a 16 ms step of the zoom: {step:?}");
	for _ in 0..200 {
		engine.frame(16.0).expect("frame");
	}
	let end = camera(&engine, "");
	// The zoom closes a share of the gap each frame, so it only nears its target.
	assert!(end[..2] == [6000.0, 5000.0] && (end[2] - 2.0).abs() < 1e-5, "arrived: {end:?}");
}
