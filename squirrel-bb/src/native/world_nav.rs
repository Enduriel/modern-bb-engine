//! The world-map pathfinder (`World.getNavigator().findPath`), reproducing the original
//! engine's `WorldNavigator::findPath` exactly — recovered from its disassembly
//! (2026-10-13) and verified against 140 routes captured from the original game.
//!
//! Exactness matters: worldgen builds roads along these routes, and ties between
//! equal-cost routes are broken by the details below.
//!
//! * Integer costs: the settings store `ActionPointCosts` as truncated ints (default 2
//!   each); the search uses `cost * 10` per tile. A step costs
//!   `trunc(cost * (road ? RoadMult : 1.0) + heat * HeatCost)` (in `f32`).
//! * `f = g + 20 * hexDistance(tile, goal)`; the open list is a binary heap ordered by
//!   `f` only, with MSVC `push_heap`/`pop_heap` semantics and a full `make_heap` after a
//!   node's cost is lowered.
//! * Neighbours are generated in the order N, S, NE, NW, SW, SE, skipping the direction
//!   straight back to where a node was reached from.
//! * The goal is detected when it is *generated* as a neighbour (before any cost or
//!   closed check), as is any road tile with `StopAtRoad`.

/// What the search needs to know about one tile.
pub(crate) struct NavTile {
	pub id: usize,
	pub type_: usize,
	/// `RoadDirections != 0`.
	pub road: bool,
	/// `HeatFromSettlements` (stored as a byte in the original).
	pub heat: u8,
}

/// The navigator settings object (`createSettings()`), as the original stores it.
pub(crate) struct NavSettings {
	pub costs: [i32; 32],
	pub road_mult: f32,
	pub heat_cost: i32,
	pub road_only: bool,
	pub stop_at_road: bool,
}

impl Default for NavSettings {
	fn default() -> Self {
		NavSettings {
			costs: [2; 32],
			road_mult: 0.0,
			heat_cost: 0,
			road_only: false,
			stop_at_road: false,
		}
	}
}

/// (axial dq, dr, the direction stored on the new node, skip when arrived via).
const BLOCKS: [(i64, i64, u8, u8); 6] = [
	(0, 1, 0, 3),  // N
	(0, -1, 3, 0), // S
	(1, 0, 1, 4),  // NE
	(-1, 1, 5, 2), // NW
	(-1, 0, 4, 1), // SW
	(1, -1, 2, 5), // SE
];
const NO_DIR: u8 = 6;

struct Node {
	f: i32,
	g: i32,
	q: i64,
	r: i64,
	id: usize,
	parent: Option<usize>,
	dir: u8,
}

/// x86 `cvttss2si`: truncate toward zero; NaN or out of `i32` range gives `i32::MIN`
/// (the "integer indefinite"). The original converts costs this way, so an infinite
/// cost (speed 0) becomes `i32::MIN`, which `* 10` wraps to 0: impassable.
pub(crate) fn cvtt_i32(value: f64) -> i32 {
	if value.is_nan() || value >= 2_147_483_648.0 || value < -2_147_483_648.0 {
		i32::MIN
	} else {
		value as i32
	}
}

fn hex_distance(aq: i64, ar: i64, bq: i64, br: i64) -> i64 {
	let (dq, dr) = (aq - bq, ar - br);
	(dq.abs() + dr.abs() + (dq + dr).abs()) / 2
}

/// MSVC `std::` heap primitives over node indices, with the original's comparator
/// `less(a, b) = a.f > b.f` (a min-heap on `f`, no secondary key).
struct Heap<'a> {
	nodes: &'a [Node],
}

impl Heap<'_> {
	fn less(&self, a: usize, b: usize) -> bool {
		self.nodes[a].f > self.nodes[b].f
	}

	fn push_by_index(&self, h: &mut [usize], mut hole: usize, top: usize, val: usize) {
		while top < hole {
			let idx = (hole - 1) / 2;
			if !self.less(h[idx], val) {
				break;
			}
			h[hole] = h[idx];
			hole = idx;
		}
		h[hole] = val;
	}

	fn adjust(&self, h: &mut [usize], mut hole: usize, bottom: usize, val: usize) {
		let top = hole;
		let mut idx = hole;
		// `bottom >= 1` at every call site (pop of >= 2 elements, make of >= 2).
		let max_non_leaf = (bottom - 1) / 2;
		while idx < max_non_leaf {
			idx = 2 * idx + 2;
			if self.less(h[idx], h[idx - 1]) {
				idx -= 1;
			}
			h[hole] = h[idx];
			hole = idx;
		}
		if idx == max_non_leaf && bottom % 2 == 0 {
			h[hole] = h[bottom - 1];
			hole = bottom - 1;
		}
		self.push_by_index(h, hole, top, val);
	}

	fn make(&self, h: &mut [usize]) {
		let n = h.len();
		let mut hole = n / 2;
		while hole > 0 {
			hole -= 1;
			let val = h[hole];
			self.adjust(h, hole, n, val);
		}
	}

	fn push(&self, h: &mut Vec<usize>, node: usize) {
		h.push(node);
		let last = h.len() - 1;
		self.push_by_index(h, last, 0, node);
	}

	fn pop(&self, h: &mut Vec<usize>) -> usize {
		let n = h.len();
		if n >= 2 {
			let val = h[n - 1];
			h[n - 1] = h[0];
			self.adjust(h, 0, n - 1, val);
		}
		h.pop().expect("pop on a non-empty heap")
	}
}

/// The path from axial `start` to `goal`: the waypoint tile ids after the start, ending
/// at the goal (or, with `max_len`/`StopAtRoad`, where the search stopped). Empty if
/// there is none. `tile(q, r)` resolves an in-grid axial coordinate.
pub(crate) fn find_path(
	start: (i64, i64),
	goal: (i64, i64),
	max_len: i64,
	settings: &NavSettings,
	tile: impl Fn(i64, i64) -> Option<NavTile>,
) -> Vec<usize> {
	if start == goal {
		return Vec::new();
	}
	let (Some(start_tile), Some(goal_tile)) = (tile(start.0, start.1), tile(goal.0, goal.1)) else {
		return Vec::new();
	};
	if max_len == 0 && goal_tile.type_ == 0 {
		return Vec::new();
	}

	let mut nodes: Vec<Node> = Vec::new();
	let mut node_of: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
	let mut open: std::collections::HashSet<usize> = std::collections::HashSet::new();
	let mut closed: std::collections::HashSet<usize> = std::collections::HashSet::new();
	let mut heap: Vec<usize> = Vec::new();

	nodes.push(Node {
		f: 0,
		g: 0,
		q: start.0,
		r: start.1,
		id: start_tile.id,
		parent: None,
		dir: NO_DIR,
	});
	node_of.insert(start_tile.id, 0);
	open.insert(start_tile.id);
	heap.push(0);

	while !heap.is_empty() {
		let cur = Heap { nodes: &nodes }.pop(&mut heap);
		let (cq, cr, cdir, cg) = {
			let c = &nodes[cur];
			(c.q, c.r, c.dir, c.g)
		};
		for (dq, dr, dir, skip) in BLOCKS {
			if cdir == skip {
				continue;
			}
			let (nq, nr) = (cq + dq, cr + dr);
			let Some(nt) = tile(nq, nr) else {
				continue;
			};
			if nt.id == goal_tile.id || (settings.stop_at_road && nt.road) {
				return reconstruct(&nodes, cur, goal_tile.id);
			}
			if closed.contains(&nt.id) {
				continue;
			}
			let cost = settings.costs.get(nt.type_).map_or(0, |c| c.wrapping_mul(10));
			if cost == 0 || (settings.road_only && !nt.road) {
				closed.insert(nt.id);
				continue;
			}
			let dist = hex_distance(nq, nr, goal.0, goal.1);
			if max_len != 0 && dist <= max_len && dist != 1 {
				return reconstruct(&nodes, cur, nt.id);
			}
			let mult = if nt.road { settings.road_mult } else { 1.0 };
			let heat = i32::from(nt.heat).wrapping_mul(settings.heat_cost);
			let step = cvtt_i32(f64::from(cost as f32 * mult + heat as f32));
			let g = cg.wrapping_add(step);
			let f = g.wrapping_add(20 * dist as i32);
			if !open.contains(&nt.id) {
				let idx = nodes.len();
				nodes.push(Node {
					f,
					g,
					q: nq,
					r: nr,
					id: nt.id,
					parent: Some(cur),
					dir,
				});
				node_of.insert(nt.id, idx);
				open.insert(nt.id);
				Heap { nodes: &nodes }.push(&mut heap, idx);
			} else {
				let idx = node_of[&nt.id];
				if g < nodes[idx].g {
					let n = &mut nodes[idx];
					n.g = g;
					n.f = f;
					n.parent = Some(cur);
					n.dir = dir;
					Heap { nodes: &nodes }.make(&mut heap);
				}
			}
		}
		let id = nodes[cur].id;
		closed.insert(id);
		open.remove(&id);
	}
	Vec::new()
}

/// `[.., parent, current, end]`: the current node's ancestry without the start, then
/// the tile the search stopped on.
fn reconstruct(nodes: &[Node], current: usize, end: usize) -> Vec<usize> {
	let mut path = vec![end];
	let mut n = current;
	while let Some(parent) = nodes[n].parent {
		path.push(nodes[n].id);
		n = parent;
	}
	path.reverse();
	path
}
