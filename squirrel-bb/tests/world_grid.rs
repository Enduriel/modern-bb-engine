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
