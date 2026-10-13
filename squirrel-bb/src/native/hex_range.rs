//! The original's in-range walk over its hex grids (the hex-grid helper class both the
//! world map and the battle map use), which `World.queryTilesInRange`,
//! `Tactical.queryTilesInRange` and `Tactical.queryActorsInRange` iterate in.
//!
//! From the centre it enters the six sectors in direction order 0..5; sector `d` spreads
//! through directions d and d+1 (sector 5 through 5 and 0, trying 5 first), depth first.
//! Each tile within range is taken the first time the walk reaches it; the centre is
//! never taken. The sectors partition the map, so a tile already taken has had its
//! whole sub-walk done: skipping it gives the original's order without its repeated
//! walks. Checked against the original's tactical queries (70 recorded queries, see
//! `tactical_nav/ground_truth.rs`).

/// The axial step for each direction.
const STEP: [(i64, i64); 6] = [(0, 1), (1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1)];
/// The directions each sector spreads through, in the order tried.
const SECTOR: [[usize; 2]; 6] = [[0, 1], [1, 2], [2, 3], [3, 4], [4, 5], [5, 0]];

fn distance((aq, ar): (i64, i64), (bq, br): (i64, i64)) -> i64 {
	let (dq, dr) = (aq - bq, ar - br);
	(dq.abs() + dr.abs() + (dq + dr).abs()) / 2
}

/// The tiles within `max` of the tile at axial `centre` (the centre excluded), in the
/// original's order. `idx` maps axial coords to a tile id (`None` off the map), ids
/// below `n_tiles`.
pub(crate) fn tiles_in_range(centre: (i64, i64), max: i64, n_tiles: usize, idx: impl Fn(i64, i64) -> Option<usize>) -> Vec<usize> {
	let mut walk = Walk {
		centre,
		max,
		idx: &idx,
		taken: vec![false; n_tiles],
		out: Vec::new(),
	};
	for (d, dirs) in SECTOR.iter().enumerate() {
		let at = (centre.0 + STEP[d].0, centre.1 + STEP[d].1);
		if let Some(id) = idx(at.0, at.1) {
			// The first ring is entered whatever its answer (its children are not).
			walk.spread(at, id, distance(centre, at) <= max, dirs);
		}
	}
	walk.out
}

struct Walk<'a, F: Fn(i64, i64) -> Option<usize>> {
	centre: (i64, i64),
	max: i64,
	idx: &'a F,
	taken: Vec<bool>,
	out: Vec<usize>,
}

impl<F: Fn(i64, i64) -> Option<usize>> Walk<'_, F> {
	fn spread(&mut self, at: (i64, i64), id: usize, in_range: bool, dirs: &[usize; 2]) {
		if self.taken[id] {
			return;
		}
		if in_range {
			self.taken[id] = true;
			self.out.push(id);
		}
		for &d in dirs {
			let next = (at.0 + STEP[d].0, at.1 + STEP[d].1);
			if let Some(n) = (self.idx)(next.0, next.1) {
				if distance(self.centre, next) <= self.max {
					self.spread(next, n, true, dirs);
				}
			}
		}
	}
}
