//! The original's ray walker over the battle map (its hex-grid helper's walk with
//! flag 0), which vision and line of sight run on.
//!
//! From the start tile it steps into each direction in `dirs` (bit order 0..5). Where
//! the callback answers 1 it spreads on through two cones, {d, d+1} then {d, d-1}: each
//! spread visits, depth first and in the direction order 0, 3, 1, 5, 4, 2, every
//! not-yet-visited neighbour in its two directions, recursing unless the callback
//! answers 0 (stop) or 3 (blocks the view). A tile is visited once per walk, so which
//! path reaches it first decides whether it is seen.

use super::TacticalGridState;

/// The axial step for each direction.
const STEP: [(i64, i64); 6] = [(0, 1), (1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1)];
/// The order the spread tries its directions in.
const ORDER: [usize; 6] = [0, 3, 1, 5, 4, 2];
/// The two cones a ray in direction `d` spreads through.
const CONE_A: [u8; 6] = [0x03, 0x06, 0x0c, 0x18, 0x30, 0x21];
const CONE_B: [u8; 6] = [0x21, 0x03, 0x06, 0x0c, 0x18, 0x30];

/// The callback's answer: stop here, go on, or this tile blocks what's behind it.
pub(crate) const STOP: u32 = 0;
pub(crate) const GO_ON: u32 = 1;
pub(crate) const BLOCKS: u32 = 3;

fn step(g: &TacticalGridState, id: usize, d: usize) -> Option<usize> {
	let t = &g.tiles[id];
	g.idx_checked(t.q + STEP[d].0, t.r + STEP[d].1)
}

/// Walk from `start` through the directions in `dirs`, calling `visit(grid, tile)`.
pub(crate) fn walk_rays(g: &mut TacticalGridState, start: usize, dirs: u8, mut visit: impl FnMut(&mut TacticalGridState, usize) -> u32) {
	let mut visited = vec![false; g.tiles.len()];
	for d in 0..6 {
		if dirs & (1 << d) == 0 {
			continue;
		}
		let Some(n) = step(g, start, d) else { continue };
		if visit(g, n) == GO_ON {
			spread(g, n, CONE_A[d], &mut visited, &mut visit);
			spread(g, n, CONE_B[d], &mut visited, &mut visit);
		}
	}
}

fn spread(
	g: &mut TacticalGridState,
	from: usize,
	cone: u8,
	visited: &mut [bool],
	visit: &mut impl FnMut(&mut TacticalGridState, usize) -> u32,
) {
	visited[from] = true;
	for d in ORDER {
		if cone & (1 << d) == 0 {
			continue;
		}
		let Some(n) = step(g, from, d) else { continue };
		if visited[n] {
			continue;
		}
		let r = visit(g, n);
		visited[n] = true;
		if r != STOP && r != BLOCKS {
			spread(g, n, cone, visited, visit);
		}
	}
}
