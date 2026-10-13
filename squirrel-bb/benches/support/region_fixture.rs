//! A recorded `World.findNextRegion` workload: every call one worldgen makes (32k, from
//! `buildLabels`), each on the grid as it was at that call, with the result the engine
//! returned. Shared by the `world_search` bench and the flood tests, which compile
//! `src/native/world_search.rs` directly (it depends only on `std`).
//!
//! `benches/data/regions_rust-engine-test.txt` comes from `boot_world --seed
//! rust-engine-test --setup benches/data/region_capture_setup.nut`, keeping the stderr
//! lines starting with `GRID`, `ROW`, `PASS`, `REGION` and `F` (the format is described
//! in the script).

// Each including crate uses a different subset.
#![allow(dead_code)]

use crate::world_search;

#[derive(Clone, Copy)]
struct Tile {
	type_: i64,
	region: i64,
}

/// The tile data `findNextRegion` reads, in square coordinates.
#[derive(Clone)]
pub struct Grid {
	width: i64,
	height: i64,
	tiles: Vec<Tile>,
}

impl Grid {
	fn idx(&self, q: i64, r: i64) -> Option<usize> {
		let (col, row) = (q, r + q / 2);
		(col >= 0 && col < self.width && row >= 0 && row < self.height)
			.then(|| (row * self.width + col) as usize)
	}

	/// `World.findNextRegion(tile, dist, type)` at square `(col, row)`, as the native
	/// computes it.
	pub fn find_next_region(&self, col: i64, row: i64, dist: i64, type_: i64) -> i64 {
		let center = (col, row - col / 2);
		world_search::flood_last(
			center,
			dist,
			|q, r| self.idx(q, r).is_some(),
			|q, r| {
				let t = &self.tiles[self.idx(q, r)?];
				(t.region != 0 && (type_ == 0 || t.type_ == type_)).then_some(t.region)
			},
		)
		.unwrap_or(0)
	}

	fn type_at(&self, col: i64, row: i64) -> i64 {
		self.tiles[(row * self.width + col) as usize].type_
	}

	fn set_region(&mut self, col: i64, row: i64, region: i64) {
		self.tiles[(row * self.width + col) as usize].region = region;
	}
}

pub struct Call {
	pub col: i64,
	pub row: i64,
	pub dist: i64,
	pub type_: i64,
	/// What the engine returned during the recorded worldgen.
	pub result: i64,
	/// The Region worldgen gave the tile afterwards, when it is not `result`.
	region_after: Option<i64>,
}

/// One `buildLabels` pass: the grid at its first call, then its calls.
struct Pass {
	grid: Grid,
	calls: Vec<Call>,
}

pub struct RegionFixture {
	passes: Vec<Pass>,
}

impl RegionFixture {
	pub fn load(path: &str) -> RegionFixture {
		let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
		let int = |s: &str| s.parse::<i64>().unwrap();
		let mut passes: Vec<Pass> = Vec::new();
		let mut grid = None;
		let mut dist = 0;
		for line in text.lines() {
			let f: Vec<&str> = line.split(' ').collect();
			match f[0] {
				"GRID" => {
					grid = Some(Grid {
						width: int(f[1]),
						height: int(f[2]),
						tiles: Vec::new(),
					})
				}
				"ROW" => grid.as_mut().unwrap().tiles.extend(f[1..].iter().map(|c| {
					let mut v = c.split(':').map(int);
					Tile {
						type_: v.next().unwrap(),
						region: v.next().unwrap_or(0),
					}
				})),
				"PASS" => {
					dist = int(f[1]);
					let grid = grid.take().unwrap();
					assert_eq!(grid.tiles.len() as i64, grid.width * grid.height);
					passes.push(Pass {
						grid,
						calls: Vec::new(),
					});
				}
				"REGION" => {
					let call = passes.last_mut().unwrap().calls.last_mut().unwrap();
					assert_eq!(
						(call.col, call.row),
						(int(f[1]), int(f[2])),
						"REGION follows its call"
					);
					call.region_after = Some(int(f[3]));
				}
				"F" => {
					let pass = passes.last_mut().unwrap();
					let (col, row) = (int(f[1]), int(f[2]));
					pass.calls.push(Call {
						col,
						row,
						dist,
						type_: f
							.get(4)
							.map_or_else(|| pass.grid.type_at(col, row), |t| int(t)),
						result: int(f[3]),
						region_after: None,
					});
				}
				_ => {}
			}
		}
		RegionFixture { passes }
	}

	pub fn len(&self) -> usize {
		self.passes.iter().map(|p| p.calls.len()).sum()
	}

	/// Replay every call in order on the evolving grid, handing `each` the grid as the
	/// call saw it and the call. The grid then takes the Region worldgen gave the tile,
	/// whatever `each` computes, so one wrong answer cannot cascade.
	pub fn replay(&self, mut each: impl FnMut(&Grid, &Call)) {
		for pass in &self.passes {
			let mut grid = pass.grid.clone();
			for call in &pass.calls {
				each(&grid, call);
				grid.set_region(call.col, call.row, call.region_after.unwrap_or(call.result));
			}
		}
	}
}
