//! The World hex tile grid, driven through Squirrel: resize, square/axial coords,
//! tile fields persisting in the Rust grid, neighbours, distance, and surrounding
//! counts.

use squirrel::SQVM;
use squirrel_bb::{WorldGrid, install_natives};

fn vm_with_grid() -> (SQVM, WorldGrid) {
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm); // creates the World table
	let grid = WorldGrid::new();
	grid.install(&mut vm);
	(vm, grid)
}

#[test]
fn resize_coords_fields_and_neighbours() {
	let (vm, grid) = vm_with_grid();
	vm.run_script(
		r#"
		World.resizeScene(10, 8);
		assert(World.getMapSize().X == 10 && World.getMapSize().Y == 8);
		assert(World.isValidTileSquare(0, 0));
		assert(!World.isValidTileSquare(10, 0));
		assert(!World.isValidTileSquare(0, 8));

		local t = World.getTileSquare(4, 4);
		assert(t.SquareCoords.X == 4 && t.SquareCoords.Y == 4);
		assert(t.Coords.X == 4 && t.Coords.Y == 2);   // q = col, r = row - col/2 = 4 - 2
		assert(t.X == 4 && t.Y == 2);                  // axial scalars

		// Field write persists in the Rust grid (handles alias the same tile).
		t.Type = 5;
		assert(World.getTileSquare(4, 4).Type == 5);
		assert(!t.HasRoad);
		t.RoadDirections = 3;
		assert(t.HasRoad);                              // derived from RoadDirections

		// axial getTile round-trips to the same square tile
		local a = World.getTile(4, 2);
		assert(a.SquareCoords.X == 4 && a.SquareCoords.Y == 4);
		assert(a.isSameTileAs(t));

		// neighbour N = axial (0,+1) -> square (4,5); adjacent (distance 1)
		assert(t.hasNextTile(0));
		local n = t.getNextTile(0);
		assert(n.SquareCoords.X == 4 && n.SquareCoords.Y == 5);
		assert(t.getDistanceTo(n) == 1);
		assert(t.getDistanceTo(t) == 0);

		// an edge tile has fewer valid neighbours
		local edge = World.getTileSquare(0, 0);
		assert(!edge.hasNextTile(4));                  // SW off-grid

		// clearTiles resets gameplay fields but not dims
		World.clearTiles();
		assert(World.getTileSquare(4, 4).Type == 0);
		assert(World.getMapSize().X == 10);

		// out-of-bounds tile access returns null
		assert(World.getTileSquare(99, 99) == null);
		"#,
		"world_grid_basic",
	)
	.expect("world grid basics");

	// Host-side inspection sees the same grid.
	assert_eq!(grid.dims(), (10, 8));
	grid.clear();
}

#[test]
fn surrounding_tiles_of_type() {
	let (vm, grid) = vm_with_grid();
	vm.run_script(
		r#"
		World.resizeScene(10, 10);
		local c = World.getTileSquare(5, 5);   // interior: all 6 neighbours valid
		c.getNextTile(0).Type = 7;
		c.getNextTile(1).Type = 7;
		assert(c.getSurroundingTilesOfType(7) == 2);
		assert(c.getSurroundingTilesNotOfType(7) == 4);

		// getNumOfTilesWithType scans the whole grid
		assert(World.getNumOfTilesWithType([7]) == 2);
		assert(World.getNumOfTilesWithType([0]) == 98);   // 100 - 2
		"#,
		"world_grid_surrounding",
	)
	.expect("surrounding counts");
	grid.clear();
}

#[test]
fn direction8_between_tiles_follows_their_world_positions() {
	let (vm, grid) = vm_with_grid();
	vm.run_script(
		r#"
		World.resizeScene(10, 10);
		local c = World.getTileSquare(5, 5);
		assert(c.getDirection8To(c.getNextTile(0)) == 0);        // N: straight up the map
		assert(c.getDirection8To(c.getNextTile(3)) == 4);        // S
		assert(c.getDirection8To(World.getTileSquare(7, 5)) == 2); // E: same row parity, dy = 0
		assert(c.getDirection8To(World.getTileSquare(3, 5)) == 6); // W
		assert(c.getDirection8To(World.getTileSquare(7, 7)) == 1); // NE: 360 px across, 240 up
		"#,
		"world_grid_direction8",
	)
	.expect("tile directions");
	grid.clear();
}

#[test]
fn stub_visual_methods_are_noops() {
	let (vm, grid) = vm_with_grid();
	vm.run_script(
		r#"
		World.resizeScene(4, 4);
		local t = World.getTileSquare(1, 1);
		// Visual/detail methods accept any args and don't throw (no-op stubs).
		t.clear();
		t.setBrush("ground.brush");
		t.spawnDetail("rock", 0, 1.0);
		t.clearAllBut(3);
		t.setColor(123);
		"#,
		"world_grid_stubs",
	)
	.expect("stub methods are no-ops");
	grid.clear();
}

#[test]
fn details_without_a_mirroring_argument_are_mirrored_at_random() {
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm);
	let grid = WorldGrid::new();
	grid.set_engine_rng(squirrel_bb::EngineRng::seeded(42));
	grid.install(&mut vm);
	vm.run_script(
		r#"
		World.resizeScene(4, 4);
		local t = World.getTileSquare(1, 1);
		for (local i = 0; i < 40; i++) t.spawnDetail("stone", -10, 0);
		for (local i = 0; i < 10; i++) t.spawnDetail("kept", -10, 0, false);
		"#,
		"flips",
	)
	.expect("spawn");
	let d = details(&grid, 1, 1);
	let flipped = d.iter().filter(|d| &*d.brush == "stone" && d.flipped).count();
	assert!(flipped > 5 && flipped < 35, "{flipped} of 40 mirrored");
	assert!(d.iter().filter(|d| &*d.brush == "kept").all(|d| !d.flipped));
	grid.clear();
}

/// The details of a tile as the renderer sees them, by brush.
fn details(grid: &WorldGrid, col: i64, row: i64) -> Vec<squirrel_bb::scene::SceneDetail> {
	let tile = grid.scene_tiles().into_iter().find(|t| t.col == col && t.row == row).expect("tile");
	tile.details
}

fn brushes(details: &[squirrel_bb::scene::SceneDetail]) -> Vec<&str> {
	details.iter().map(|d| &*d.brush).collect()
}

#[test]
fn details_stay_on_their_tile_for_the_renderer() {
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm);
	let grid = WorldGrid::new();
	grid.set_engine_rng(squirrel_bb::EngineRng::seeded(1));
	grid.install(&mut vm);
	vm.run_script(
		r#"
		World.resizeScene(4, 4);
		local t = World.getTileSquare(1, 2);
		local tree = t.spawnDetail("tree", -10, 32);
		tree.Alpha = 120;
		tree.Scale = 0.85;
		t.spawnDetail("hill", 30, 8, true);
		t.spawnDetail("house", -13, 128);
		tree.setBrush("tree_2");          // changes after spawning reach the tile
		"#,
		"details",
	)
	.expect("spawn details");
	let d = details(&grid, 1, 2);
	assert_eq!(brushes(&d), ["tree_2", "hill", "house"]);
	assert_eq!((d[0].z_level, d[0].flags, d[0].alpha, d[0].scale), (-10, 32, 120, 0.85));
	assert!(d[1].flipped, "the 4-argument form mirrors as asked");
	assert!(details(&grid, 0, 0).is_empty());

	vm.run_script("World.getTileSquare(1, 2).clear(8);", "clear mask").expect("clear");
	assert_eq!(brushes(&details(&grid, 1, 2)), ["tree_2", "house"]);
	vm.run_script("World.getTileSquare(1, 2).clearAllBut(128);", "clear all but").expect("clear");
	assert_eq!(brushes(&details(&grid, 1, 2)), ["house"]);
	vm.run_script("World.getTileSquare(1, 2).clear();", "clear all").expect("clear");
	assert!(details(&grid, 1, 2).is_empty());
	grid.clear();
}

/// What a renderer that keeps the map learns: the map's revision moves when what it
/// draws changes in its order (terrain, details added or removed, the map sized), not
/// for gameplay fields; a detail's look changing is logged once until taken; and the
/// whole map keeps a hidden detail, marked.
#[test]
fn a_renderer_learns_what_changed_on_the_map() {
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm);
	let grid = WorldGrid::new();
	grid.set_engine_rng(squirrel_bb::EngineRng::seeded(1));
	grid.install(&mut vm);
	let run = |code: &str| vm.run_script(code, "map").expect("script");
	run("World.resizeScene(4, 4); ::t <- World.getTileSquare(1, 2); ::tree <- t.spawnDetail(\"tree\", -10, 32);");
	let mut revision = grid.map_revision();
	let mut moved = |code: &str| {
		run(code);
		let now = grid.map_revision();
		std::mem::replace(&mut revision, now) != now
	};
	assert!(!moved("t.Type = 3; t.IsDiscovered = true; t.HeatFromSettlements = 2;"), "gameplay fields");
	assert!(moved("t.setBrush(\"plains\");"), "terrain");
	assert!(moved("t.IsFlipped = true;"), "mirroring");
	assert!(moved("t.spawnDetail(\"hill\", 30, 8, true);"), "a detail added");
	assert!(moved("t.clear(8);"), "a detail removed");

	grid.take_detail_changes();
	assert!(!moved("tree.Alpha = 50; tree.Scale = 0.5; tree.Visible = false;"), "a detail's look keeps the order");
	let changed = grid.take_detail_changes();
	assert_eq!(changed.len(), 1, "logged once");
	assert_eq!((&*changed[0].brush, changed[0].alpha, changed[0].scale, changed[0].hidden), ("tree", 50, 0.5, true));
	assert!(grid.take_detail_changes().is_empty(), "taken");

	let map = grid.scene_map();
	let tile = map.iter().find(|t| t.col == 1 && t.row == 2).expect("tile");
	assert_eq!((tile.details.len(), tile.details[0].hidden, tile.details[0].id), (1, true, changed[0].id), "kept, hidden");
	assert!(details(&grid, 1, 2).is_empty(), "the frame's scene leaves it out");
	grid.clear();
}

#[test]
fn tile_brush_is_state_like_the_original() {
	let mut vm = squirrel::SQVM::new(1024);
	squirrel_bb::install_natives(&mut vm);
	let grid = squirrel_bb::WorldGrid::new();
	grid.install(&mut vm);
	vm.run_script(
		r#"
		World.resizeScene(4, 4);
		local t = World.getTileSquare(1, 1);
		assert(typeof t.getBrush() == "string" && t.getBrush() == "");
		t.setBrush("world_highlands_03");
		assert(t.getBrush() == "world_highlands_03");
		assert(World.getTileSquare(1, 1).getBrush() == "world_highlands_03"); // handles alias
		t.resetBrush();
		assert(t.getBrush() == "");
		t.setBrush("x");
		World.clearTiles();
		assert(t.getBrush() == "");
		"#,
		"brush",
	)
	.expect("brush round-trips");
	grid.clear();
}

/// Routes captured from the ORIGINAL game's `findPath` (2026-10-13) on a
/// 140x170 all-plains map and on a deterministic mixed-cost map. The navigator must
/// reproduce them exactly: worldgen builds roads along these routes.
#[test]
fn find_path_matches_the_original_routes() {
	let mut vm = squirrel::SQVM::new(1024);
	squirrel_bb::install_natives(&mut vm);
	let grid = squirrel_bb::WorldGrid::new();
	grid.install(&mut vm);
	vm.run_script(
		r#"
		::Const <- { World = { TerrainTypeNavCost = [0] } };
		foreach (s in [0.0, 1.0, 0.5, 0.65, 0.65, 0.65, 0.65, 0.65, 0.25, 0.5, 1.0, 0.75, 0.9, 0.9, 1.0, 0.5, 0.75, 0.9])
			::Const.World.TerrainTypeNavCost.push(10 * (1.0 / s)); // speed 0 -> inf, like the game
		World.resizeScene(140, 170);
		local function setMap( _mixed )
		{
			for (local x = 0; x < 140; x++)
				for (local y = 0; y < 170; y++)
				{
					local t = World.getTileSquare(x, y);
					t.RoadDirections = 0;
					t.Type = _mixed ? [2, 2, 2, 4, 5, 10, 12, 13, 9][(x * 7 + y * 13 + (x * y) % 5) % 9] : 2;
				}
		}
		local function path( _fx, _fy, _tx, _ty )
		{
			local nav = World.getNavigator();
			local s = nav.createSettings();
			s.ActionPointCosts = ::Const.World.TerrainTypeNavCost;
			local p = nav.findPath(World.getTileSquare(_fx, _fy), World.getTileSquare(_tx, _ty), s, 0);
			local out = "";
			while (!p.isEmpty()) { local c = p.getCurrent(); local t = World.getTile(c.X, c.Y); out += t.SquareCoords.X + "," + t.SquareCoords.Y + " "; p.pop(); }
			return out;
		}
		setMap(false);
		assert(path(91, 19, 101, 32) == "92,20 93,20 94,21 94,22 94,23 94,24 94,25 95,25 95,26 95,27 96,28 97,28 98,29 99,29 100,30 101,30 101,31 101,32 ");
		assert(path(26, 75, 50, 100) == "27,75 28,76 29,76 30,77 31,77 32,78 33,78 34,79 35,79 36,80 37,80 38,81 39,81 40,82 41,82 42,83 43,83 44,84 44,85 44,86 44,87 44,88 44,89 44,90 44,91 44,92 44,93 45,93 46,94 47,94 48,95 49,95 49,96 49,97 49,98 49,99 50,100 ");
		assert(path(65, 113, 88, 118) == "66,114 67,114 68,115 69,115 70,116 71,116 72,117 73,117 74,118 75,117 76,118 77,118 78,118 79,117 80,117 81,116 82,117 83,116 84,116 85,116 86,117 87,117 88,118 ");
		assert(path(66, 15, 69, 34) == "66,16 66,17 66,18 66,19 66,20 66,21 66,22 66,23 66,24 66,25 66,26 66,27 66,28 66,29 66,30 66,31 66,32 67,32 68,33 68,34 69,34 ");
		setMap(true);
		assert(path(91, 102, 100, 117) == "92,103 92,104 92,105 93,105 93,106 93,107 93,108 93,109 93,110 93,111 94,112 95,112 96,113 97,113 98,114 99,114 99,115 99,116 100,117 ");
		assert(path(25, 48, 34, 67) == "26,49 27,49 28,50 28,51 28,52 28,53 28,54 28,55 28,56 28,57 29,57 30,58 31,58 31,59 31,60 32,61 33,61 33,62 33,63 34,64 34,65 34,66 34,67 ");
		"#,
		"nav_golden",
	)
	.expect("routes match the original");
	grid.clear();
}
