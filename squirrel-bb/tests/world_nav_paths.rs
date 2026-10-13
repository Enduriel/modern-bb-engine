//! The pathfinder must stay bit-identical (worldgen builds roads along its routes, and
//! the original's tie-breaking is reproduced exactly). Compares routes, stored as one
//! FNV-1a hash per route, with what the reference implementation
//! (`support/world_nav_reference.rs`) produces:
//!
//! * every `findPath` call of a recorded worldgen, each on the grid as it was at the call
//!   (so these are the routes worldgen really took);
//! * synthetic queries on the final grid for what worldgen never exercises but world-map
//!   movement and mods do: heat cost, road multipliers, `RoadOnly`, `StopAtRoad`,
//!   `max_len`, and the early returns.
//!
//! `NAV_BLESS=1` rewrites the expected hashes from the reference implementation.

#[path = "../benches/support/nav_fixture.rs"]
mod nav_fixture;
#[allow(dead_code)]
#[path = "../src/native/world_nav.rs"]
mod world_nav;
#[allow(dead_code, clippy::manual_is_multiple_of)] // verbatim reference
#[path = "support/world_nav_reference.rs"]
mod world_nav_reference;

use nav_fixture::{Grid, NavFixture, Query};
use world_nav::NavSettings;

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/data/");

/// FNV-1a over the route's tile ids, comma-separated, then the route length.
fn route_hash(route: &[usize]) -> String {
	let text = route
		.iter()
		.map(usize::to_string)
		.collect::<Vec<_>>()
		.join(",");
	let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
		(h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
	});
	format!("{hash:016x} {}", route.len())
}

/// The route the reference implementation finds for `q` on `grid`.
fn reference_route(grid: &Grid, q: &Query, s: &NavSettings) -> Vec<usize> {
	let settings = world_nav_reference::NavSettings {
		costs: s.costs,
		road_mult: s.road_mult,
		heat_cost: s.heat_cost,
		road_only: s.road_only,
		stop_at_road: s.stop_at_road,
	};
	world_nav_reference::find_path(q.start, q.goal, q.max_len, &settings, |a, b| {
		grid.tile(a, b).map(|t| world_nav_reference::NavTile {
			id: t.id,
			type_: t.type_,
			road: t.road,
			heat: t.heat,
		})
	})
}

/// Compare `got` with the expected hashes in `file`, or rewrite them from `reference`
/// under `NAV_BLESS`.
fn check(file: &str, got: Vec<String>, reference: impl FnOnce() -> Vec<String>) {
	let path = format!("{DATA}{file}");
	if std::env::var_os("NAV_BLESS").is_some() {
		std::fs::write(&path, reference().join("\n") + "\n").unwrap();
		return;
	}
	let expected = std::fs::read_to_string(&path).unwrap();
	let expected: Vec<&str> = expected.lines().collect();
	assert_eq!(got.len(), expected.len(), "{file}: query count");
	for (i, (got, want)) in got.iter().zip(&expected).enumerate() {
		assert_eq!(got, want, "{file}: route {i} differs (hash, length)");
	}
}

#[test]
fn recorded_worldgen_routes_are_unchanged() {
	let fx = NavFixture::load(&format!("{DATA}nav_rust-engine-test.txt"));
	let mut got = Vec::new();
	fx.replay(|_, route| got.push(route_hash(&route)));
	check("nav_rust-engine-test.expected", got, || {
		(0..fx.len())
			.map(|i| {
				let q = fx.query(i);
				route_hash(&reference_route(
					&fx.grid_at(i),
					q,
					&fx.settings[q.settings],
				))
			})
			.collect()
	});
}

/// splitmix64, for reproducible synthetic queries without a dependency.
struct Rng(u64);

impl Rng {
	fn next(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
		let mut z = self.0;
		z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
		z ^ (z >> 31)
	}

	fn below(&mut self, n: u64) -> u64 {
		self.next() % n
	}
}

fn with(base: &NavSettings, change: impl FnOnce(&mut NavSettings)) -> NavSettings {
	let mut s = NavSettings {
		costs: base.costs,
		road_mult: base.road_mult,
		heat_cost: base.heat_cost,
		road_only: base.road_only,
		stop_at_road: base.stop_at_road,
	};
	change(&mut s);
	s
}

/// Axial coordinates of square `(col, row)`.
fn axial((col, row): (i64, i64)) -> (i64, i64) {
	(col, row - col / 2)
}

/// Each recorded settings set, and the variants world-map scripts derive from such sets
/// (`move_order.nut`: `RoadMult = 1 / 1.5`, `*= 0.05` on roads only, `HeatCost = 100`;
/// `contract.nut`/`settlement.nut`: `RoadOnly`), plus `StopAtRoad`, crossed with random
/// endpoints and `max_len` values, then the special cases. (The final map has no type-0
/// tiles; `world_nav_fuzz.rs` covers the type-0-goal early return.)
fn synthetic_queries(fx: &NavFixture, grid: &Grid) -> Vec<(Query, NavSettings)> {
	let mut rng = Rng(0x0005_eed0_fa11_7a75);
	let (w, h) = grid.dims();
	let square = |rng: &mut Rng| (rng.below(w as u64) as i64, rng.below(h as u64) as i64);
	let road_mult = (1.0_f64 / 1.5) as f32;
	let mut out = Vec::new();
	for base in &fx.settings {
		let variants = [
			with(base, |_| {}),
			with(base, |s| s.heat_cost = 100),
			with(base, |s| s.road_mult = road_mult),
			with(base, |s| {
				s.road_mult = road_mult * 0.05;
				s.heat_cost = 100;
			}),
			with(base, |s| s.road_only = true),
			with(base, |s| s.stop_at_road = true),
		];
		for settings in variants {
			for max_len in [0, 0, 0, 1, 2, 5, 20] {
				for _ in 0..2 {
					let q = Query {
						start: axial(square(&mut rng)),
						goal: axial(square(&mut rng)),
						max_len,
						settings: 0,
					};
					out.push((q, with(&settings, |_| {})));
				}
			}
		}
	}

	let base = &fx.settings[0];
	let some = axial((w / 2, h / 2));
	let special = [
		(some, some, 0),                     // start == goal
		((-1, 0), some, 0),                  // start off the grid
		(some, (w + 5, 0), 0),               // goal off the grid
		(some, (some.0, some.1 + 1), 0),     // adjacent
		(some, (some.0 + 1, some.1 - 1), 2), // adjacent, max_len
	];
	for (start, goal, max_len) in special {
		let q = Query {
			start,
			goal,
			max_len,
			settings: 0,
		};
		out.push((q, with(base, |_| {})));
	}
	out
}

#[test]
fn synthetic_routes_are_unchanged() {
	let fx = NavFixture::load(&format!("{DATA}nav_rust-engine-test.txt"));
	let grid = fx.grid_at(fx.len());
	let queries = synthetic_queries(&fx, &grid);
	let got = queries
		.iter()
		.map(|(q, s)| route_hash(&grid.find(q, s)))
		.collect();
	check("nav_synthetic.expected", got, || {
		queries
			.iter()
			.map(|(q, s)| route_hash(&reference_route(&grid, q, s)))
			.collect()
	});
}
