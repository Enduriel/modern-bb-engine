//! A severed head lands as the original's do (see `tactical_grid::heads`): on a free
//! neighbouring tile, which it marks `IsHeadSpawned`, lying there until the tile is
//! cleared of severed heads; and a tile's `clear(flags)` takes the pools carrying them.
//! Skips cleanly when the Steam install isn't there.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};
use squirrel::SqTable;

fn script(engine: &GameEngine, code: &str) {
	engine.run_script(code, "heads").unwrap_or_else(|e| panic!("{code}: {e}"));
}

fn int(engine: &GameEngine, name: &str) -> i64 {
	SqTable::root(engine.vm()).get::<i64>(name).unwrap_or_else(|| panic!("{name}"))
}

#[test]
fn a_head_lands_next_door_and_clears_with_its_tile() {
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
		harness::run_frame(&engine, 1000.0 / 60.0).expect("world frame");
	}
	harness::engage_nearest_hostile(&engine).expect("engage");
	for frame in 0..3_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
		let scene = engine.tactical_scene();
		if !scene.tiles.is_empty() && scene.tiles.iter().all(|t| !t.brush.is_empty()) {
			break;
		}
	}

	// A head flies off a tile in the open: it lands on a neighbour, marked.
	script(
		&engine,
		"local t = ::Tactical.getTileSquare(16, 16); \
		 ::__from <- t.ID; ::__fromTile <- t; \
		 ::__layer <- ::Const.BloodPoolDecals[::Const.BloodType.Red][1]; \
		 ::Tactical.spawnHeadEffect(t, [::__layer], ::createVec(0, 0), 0.0, \"bust_head_dead_bloodpool\"); \
		 ::__marked <- -1; ::__count <- 0; \
		 for (local d = 0; d < 6; d++) if (t.hasNextTile(d)) { local n = t.getNextTile(d); if (n.Properties.has(\"IsHeadSpawned\")) { ::__marked = n.ID; ::__markedTile <- n; ::__count++; } } \
		 if (t.Properties.has(\"IsHeadSpawned\")) ::__count += 100;",
	);
	let (from, marked) = (int(&engine, "__from"), int(&engine, "__marked"));
	assert_eq!(int(&engine, "__count"), 1, "one neighbour marked, not its own tile");
	assert!(marked >= 0 && marked != from);
	let layer = SqTable::root(engine.vm()).get::<String>("__layer").expect("the layer");

	// After its second of flight it lies on that tile, over its blood pool.
	for _ in 0..66 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	let lying = |engine: &GameEngine| {
		engine
			.tactical_scene()
			.pools
			.iter()
			.filter(|p| p.tile as i64 == marked && (p.brush == layer || p.brush == "bust_head_dead_bloodpool"))
			.count()
	};
	assert_eq!(lying(&engine), 2, "the head and its pool on the tile it landed on");

	// Clearing the tile's corpses leaves it; clearing its severed heads takes it.
	script(&engine, "::__markedTile.clear(::Const.Tactical.DetailFlag.Corpse);");
	assert_eq!(lying(&engine), 2);
	script(&engine, "::__markedTile.clear(::Const.Tactical.DetailFlag.SeveredHead);");
	assert_eq!(lying(&engine), 0, "cleared with the severed heads");

	// A corpse's blood pool goes with the tile's corpses (as eating a corpse clears it).
	script(
		&engine,
		"::Tactical.spawnPoolEffect(::Const.BloodPoolDecals[::Const.BloodType.Red][0], ::__fromTile, 1.0, 1.0, ::Const.Tactical.DetailFlag.Corpse);",
	);
	let pools_on = |engine: &GameEngine| engine.tactical_scene().pools.iter().filter(|p| p.tile as i64 == from).count();
	assert_eq!(pools_on(&engine), 1);
	script(&engine, "::__fromTile.clear(::Const.Tactical.DetailFlag.Corpse);");
	assert_eq!(pools_on(&engine), 0);
}
