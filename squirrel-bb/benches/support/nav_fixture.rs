//! A recorded pathfinder workload: every `World.getNavigator().findPath` call one
//! worldgen makes, each on the grid as it was at that call (roads and heat change as
//! worldgen runs). Shared by the `world_nav` bench and the pathfinder tests, which all
//! compile `src/native/world_nav.rs` directly (it depends only on `std`).
//!
//! `benches/data/nav_rust-engine-test.txt` comes from `boot_world --seed
//! rust-engine-test --setup benches/data/nav_capture_setup.nut`, keeping the stderr lines
//! starting with `GRID`, `ROW`, `TILE`, `SET` and `NAV`.

// Each including crate uses a different subset.
#![allow(dead_code)]

use crate::world_nav::{self, NavSettings, NavTile, cvtt_i32};

pub struct Query {
	pub start: (i64, i64),
	pub goal: (i64, i64),
	pub max_len: i64,
	/// Index into [`NavFixture::settings`].
	pub settings: usize,
}

#[derive(Clone, Copy)]
struct Tile {
	type_: i64,
	road_directions: i64,
	heat: i64,
}

/// The tile data the pathfinder reads, in square coordinates.
#[derive(Clone)]
pub struct Grid {
	width: i64,
	height: i64,
	tiles: Vec<Tile>,
}

impl Grid {
	/// The tile lookup `findPath` hands the search (same conversions as the native).
	pub fn tile(&self, q: i64, r: i64) -> Option<NavTile> {
		let (col, row) = (q, r + q / 2);
		if col < 0 || col >= self.width || row < 0 || row >= self.height {
			return None;
		}
		let id = (row * self.width + col) as usize;
		let t = self.tiles[id];
		Some(NavTile {
			id,
			type_: usize::try_from(t.type_).unwrap_or(usize::MAX),
			road: t.road_directions != 0,
			heat: t.heat as u8,
		})
	}

	pub fn dims(&self) -> (i64, i64) {
		(self.width, self.height)
	}

	pub fn find(&self, q: &Query, settings: &NavSettings) -> Vec<usize> {
		world_nav::find_path(q.start, q.goal, q.max_len, settings, |a, b| self.tile(a, b))
	}
}

/// One recorded call: the tiles that changed since the previous call, then the query.
struct Step {
	changes: Vec<(usize, Tile)>,
	query: Query,
}

pub struct NavFixture {
	initial: Grid,
	steps: Vec<Step>,
	pub settings: Vec<NavSettings>,
}

impl NavFixture {
	/// Parse the capture (see `data/nav_capture_setup.nut` for the format).
	pub fn load(path: &str) -> NavFixture {
		let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
		let int = |s: &str| s.parse::<i64>().unwrap();
		let mut fixture = NavFixture {
			initial: Grid {
				width: 0,
				height: 0,
				tiles: Vec::new(),
			},
			steps: Vec::new(),
			settings: Vec::new(),
		};
		let mut changes = Vec::new();
		for line in text.lines() {
			let f: Vec<&str> = line.split(' ').collect();
			match f[0] {
				"GRID" => {
					assert!(
						fixture.steps.is_empty(),
						"the grid is captured once, before the first call"
					);
					fixture.initial.width = int(f[1]);
					fixture.initial.height = int(f[2]);
				}
				"ROW" => fixture
					.initial
					.tiles
					.extend(f[1..].iter().map(|c| parse_cell(c))),
				"TILE" => {
					let id = int(f[2]) * fixture.initial.width + int(f[1]);
					changes.push((id as usize, parse_cell(f[3])));
				}
				"SET" => {
					assert_eq!(
						int(f[1]) as usize,
						fixture.settings.len(),
						"SET ids in order"
					);
					fixture.settings.push(parse_settings(&f[2..]));
				}
				"NAV" => fixture.steps.push(Step {
					changes: std::mem::take(&mut changes),
					query: Query {
						start: (int(f[1]), int(f[2])),
						goal: (int(f[3]), int(f[4])),
						max_len: int(f[5]),
						settings: int(f[6]) as usize,
					},
				}),
				_ => {}
			}
		}
		let g = &fixture.initial;
		assert_eq!(g.tiles.len() as i64, g.width * g.height);
		fixture
	}

	pub fn len(&self) -> usize {
		self.steps.len()
	}

	pub fn query(&self, i: usize) -> &Query {
		&self.steps[i].query
	}

	/// Replay every call in order on the evolving grid, handing each route to `each`.
	pub fn replay(&self, mut each: impl FnMut(usize, Vec<usize>)) {
		let mut grid = self.initial.clone();
		for (i, step) in self.steps.iter().enumerate() {
			apply(&mut grid, &step.changes);
			each(
				i,
				grid.find(&step.query, &self.settings[step.query.settings]),
			);
		}
	}

	/// The grid as call `i` saw it (`len()` gives the grid after the last call).
	pub fn grid_at(&self, i: usize) -> Grid {
		let mut grid = self.initial.clone();
		for step in self.steps.iter().take(i + 1) {
			apply(&mut grid, &step.changes);
		}
		grid
	}
}

fn apply(grid: &mut Grid, changes: &[(usize, Tile)]) {
	for &(id, tile) in changes {
		grid.tiles[id] = tile;
	}
}

/// `type` or `type:roadDirections:heat`.
fn parse_cell(cell: &str) -> Tile {
	let v: Vec<i64> = cell.split(':').map(|n| n.parse().unwrap()).collect();
	Tile {
		type_: v[0],
		road_directions: v.get(1).copied().unwrap_or(0),
		heat: v.get(2).copied().unwrap_or(0),
	}
}

/// `roadMult heatCost stopAtRoad roadOnly costs,…`, converted the way the native's
/// `nav_settings` converts the script table.
fn parse_settings(f: &[&str]) -> NavSettings {
	let float = |s: &str| s.parse::<f64>().unwrap();
	let mut settings = NavSettings::default();
	for (i, c) in f[4].split(',').take(32).enumerate() {
		settings.costs[i] = cvtt_i32(float(c));
	}
	settings.road_mult = float(f[0]) as f32;
	settings.heat_cost = cvtt_i32(float(f[1]));
	settings.stop_at_road = f[2] == "1";
	settings.road_only = f[3] == "1";
	settings
}
