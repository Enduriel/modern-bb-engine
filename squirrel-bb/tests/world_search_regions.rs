//! `World.findNextRegion` must keep the original's flood order (worldgen keeps the
//! last match, so the order decides the regions). Replays every call of a recorded
//! worldgen on the grid as it was at the call and checks each answer against the one
//! the engine gave during that worldgen.

#[path = "../benches/support/region_fixture.rs"]
mod region_fixture;
#[allow(dead_code)]
#[path = "../src/native/world_search.rs"]
mod world_search;

use region_fixture::RegionFixture;

#[test]
fn recorded_worldgen_regions_are_unchanged() {
	let fx = RegionFixture::load(concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/benches/data/regions_rust-engine-test.txt"
	));
	let mut n = 0;
	fx.replay(|grid, c| {
		let got = grid.find_next_region(c.col, c.row, c.dist, c.type_);
		assert_eq!(
			got, c.result,
			"call {n}: findNextRegion(({}, {}), {}, {})",
			c.col, c.row, c.dist, c.type_
		);
		n += 1;
	});
	assert_eq!(n, fx.len());
}
