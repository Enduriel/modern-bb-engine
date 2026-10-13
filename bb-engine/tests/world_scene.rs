//! A new campaign's world map as a renderer reads it (`Engine::world_scene`): every
//! land tile has terrain, decorations sit on their tiles, and the player's party has its
//! sprites in the order the scripts added them. Skips cleanly when the Steam install
//! isn't there.

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
}
