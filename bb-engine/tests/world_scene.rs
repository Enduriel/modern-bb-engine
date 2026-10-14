//! A new campaign's world map as a renderer reads it (`Engine::world_scene`): every
//! land tile has terrain, decorations sit on their tiles, the player's party has its
//! sprites in the order the scripts added them, and the fog of war lies where the
//! original's does. Skips cleanly when the Steam install isn't there.

use std::path::Path;

use bb_engine::harness::{self, STEAM_DATA_DIR};
use squirrel::{Number, SqStrongRef, SqTable};

#[test]
fn a_new_campaign_world_is_ready_to_draw() {
	let dir = Path::new(STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {STEAM_DATA_DIR} not present");
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare");
	harness::start_new_campaign(&engine).expect("start");
	let scene = engine.world_scene();

	assert_eq!(scene.tiles.len(), 140 * 170);
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
	let names: Vec<&str> = player.sprites.iter().map(|s| s.name.as_str()).collect();
	let order = ["banner", "base", "body", "bodyUp", "combat", "lighting", "zoom_banner"];
	let positions: Vec<usize> = order.iter().map(|n| names.iter().position(|m| m == n).expect(n)).collect();
	assert!(positions.is_sorted(), "added in script order: {names:?}");
	let body = player.sprites.iter().find(|s| s.name == "body").expect("body");
	assert_eq!(body.brush, "figure_player_01");
	let selection = player.sprites.iter().find(|s| s.name == "selection").expect("selection");
	assert_eq!(selection.offset, (-30.0, 30.0));
	assert!(!selection.scaling);

	// Fog of war: the start uncovers the same tiles as the original's (13,560 for this
	// seed, probed 2026-10-14). The mask spans the map to the tile past its last corner,
	// is clear around the player and still fogged where nothing was uncovered.
	assert!(scene.fog.on);
	assert_eq!(scene.tiles.iter().filter(|t| t.discovered).count(), 13_560);
	assert_eq!(scene.fog.extent, (25320.0, 20460.0));
	let mask = engine.world_grid().take_fog_patch().expect("the new map's mask");
	assert_eq!((mask.x, mask.y, mask.width, mask.height), (0, 0, 2048, 2048));
	let fog_at = |x: f32, y: f32| {
		let (i, j) = ((x / 25320.0 * 2048.0) as usize, (y / 20460.0 * 2048.0) as usize);
		mask.alpha[j * 2048 + i]
	};
	assert_eq!(fog_at(14160.0, 16740.0), 0, "clear around the player");
	assert_eq!(fog_at(16600.0, 18150.0), 255, "fogged off to the north-east");
	assert!(engine.world_grid().take_fog_patch().is_none(), "unchanged since");

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
}
