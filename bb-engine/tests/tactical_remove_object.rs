//! `tile.removeObject()` takes what stands on the tile off the map and out of the
//! battle, as the original's does (disassembled, 0x4dab60): the renderer no longer draws
//! it, so a felled tree doesn't stay drawn over the stump the script puts in its place,
//! nor over a camp's props or the company deployed where it stood. And `climbBy`, the
//! battle topbar's map level buttons, moves the camera's level within 0..3. Skips
//! cleanly when the Steam install isn't there.

use std::collections::HashMap;
use std::path::Path;

use bb_engine::harness;
use squirrel::{SqArray, SqTable};

/// A battle on its map, built and drawn once.
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
	harness::unpause_world(&engine).expect("unpause");
	for frame in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
	}
	harness::engage_nearest_hostile(&engine).expect("engage");
	for frame in 0..3_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
		let scene = engine.tactical_scene();
		if !scene.tiles.is_empty() && scene.tiles.iter().all(|t| !t.brush.is_empty()) && !scene.entities.is_empty() {
			break;
		}
	}
	engine.run_script("::Tactical.State.setPause(true);", "pause").expect("pause");
	Some(engine)
}

/// The entities drawn on each tile.
fn drawn_per_tile(engine: &harness::GameEngine) -> HashMap<usize, Vec<i64>> {
	let mut per_tile: HashMap<usize, Vec<i64>> = HashMap::new();
	for e in engine.tactical_scene().entities.iter().filter(|e| !e.flying) {
		if let Some(tile) = e.tile {
			per_tile.entry(tile).or_default().push(e.id);
		}
	}
	per_tile
}

fn answers(engine: &harness::GameEngine, code: &str) -> Vec<String> {
	engine.run_script(code, "probe").unwrap_or_else(|e| panic!("{e}\n{code}"));
	let a = SqTable::root(engine.vm()).get::<SqArray>("__out").expect("answers");
	(0..a.len())
		.map(|i| {
			a.get::<String>(i)
				.or_else(|| a.get::<squirrel::Number>(i).map(|n| n.0.to_string()))
				.or_else(|| a.get::<bool>(i).map(|b| b.to_string()))
				.unwrap_or_else(|| "null".into())
		})
		.collect()
}

#[test]
fn a_removed_object_is_gone_from_the_map_and_the_battle() {
	let Some(engine) = battle() else { return };
	// The deployment (`tactical_entity_manager`) clears the objects where the company
	// stands: every tile draws at most one entity.
	let per_tile = drawn_per_tile(&engine);
	let shared: Vec<_> = per_tile.iter().filter(|(_, ids)| ids.len() > 1).collect();
	assert!(shared.is_empty(), "tiles drawing two entities: {shared:?}");

	// A boulder felled and replaced by a stump, as the forest templates cut trees.
	let out = answers(
		&engine,
		"local tile = null; \
		 for (local x = 4; x < 28 && tile == null; x++) for (local y = 4; y < 28 && tile == null; y++) { \
		   local t = ::Tactical.getTileSquare(x, y); if (t.IsEmpty) tile = t; } \
		 ::__tile <- tile; \
		 ::__boulder <- tile.spawnObject(\"entity/tactical/objects/boulder\"); \
		 ::__seen <- null; local id = ::__boulder.getID(); \
		 ::__boulder.onFinish <- function () { ::__seen = [this.isPlacedOnMap(), ::Tactical.getEntityByID(id) != null]; }; \
		 ::__out <- [tile.SquareCoords.X, tile.SquareCoords.Y, ::__boulder.getID(), tile.IsEmpty];",
	);
	let boulder: i64 = out[2].parse().expect("id");
	assert_eq!(out[3], "false", "the boulder stands there");
	let drawn = |id: i64| engine.tactical_scene().entities.iter().any(|e| e.id == id);
	assert!(drawn(boulder), "the boulder is drawn");
	let out = answers(
		&engine,
		"local before = ::__tile.getEntity().getID(); \
		 ::__tile.removeObject(); \
		 local empty = ::__tile.IsEmpty; \
		 local placed = ::__boulder.isPlacedOnMap(); \
		 local found = ::Tactical.getEntityByID(::__boulder.getID()) != null; \
		 ::__tile.removeObject(); \
		 local stump = ::__tile.spawnObject(\"entity/tactical/objects/tree_stump1\"); \
		 ::__out <- [before, empty, placed, found, stump.getID(), ::__tile.getEntity().getID() == stump.getID()];",
	);
	assert_eq!(out[0], boulder.to_string());
	assert_eq!(out[1..4], ["true", "false", "false"], "the tile is free; the boulder is off the map and out of the battle");
	assert_eq!(out[5], "true", "the stump stands on the tile");
	// Its onFinish ran, off the map and out of the battle already (probed).
	let seen = answers(&engine, "::__out <- ::__seen == null ? [\"not run\"] : ::__seen;");
	assert_eq!(seen, ["false", "false"], "onFinish saw [placed, found by id]");
	let stump: i64 = out[4].parse().expect("id");
	assert!(!drawn(boulder), "the removed boulder isn't drawn");
	assert!(drawn(stump), "the stump is drawn");
	let tile_of = |id: i64| engine.tactical_scene().entities.iter().find(|e| e.id == id).and_then(|e| e.tile);
	let tile = tile_of(stump).expect("the stump's tile");
	assert_eq!(drawn_per_tile(&engine)[&tile], vec![stump], "only the stump is drawn there");
	engine.run_script("::__tile <- null; ::__boulder <- null;", "release").expect("release");
}

#[test]
fn the_map_level_buttons_climb_within_the_cameras_range() {
	let Some(engine) = battle() else { return };
	let out = answers(
		&engine,
		"local c = ::Tactical.getCamera(); local s = ::Tactical.State; ::__out <- []; \
		 c.Level = 0; \
		 s.topbar_options_onSwitchMapLevelUpButtonClicked(); ::__out.push(c.Level); \
		 s.topbar_options_onSwitchMapLevelUpButtonClicked(); ::__out.push(c.Level); \
		 c.climbBy(5); ::__out.push(c.Level); \
		 s.topbar_options_onSwitchMapLevelUpButtonClicked(); ::__out.push(c.Level); \
		 s.topbar_options_onSwitchMapLevelDownButtonClicked(); ::__out.push(c.Level); \
		 c.climbBy(-9); ::__out.push(c.Level); \
		 s.topbar_options_onSwitchMapLevelDownButtonClicked(); ::__out.push(c.Level); \
		 c.Level = 7; ::__out.push(c.Level); \
		 c.Level = -2; ::__out.push(c.Level); \
		 c.climbBy(1.9); ::__out.push(c.Level); \
		 local w = ::World.getCamera(); w.climbBy(2); ::__out.push(w.Level); w.Level = 3; ::__out.push(w.Level);",
	);
	assert_eq!(out, ["1", "2", "3", "3", "2", "0", "0", "3", "0", "1", "0", "0"]);
	// What's drawn follows: the scene carries the level the buttons left.
	engine.run_script("::Tactical.getCamera().Level = 0; ::Tactical.getCamera().climbBy(2);", "climb").expect("climb");
	assert_eq!(engine.tactical_scene().camera.level, 2);
}
