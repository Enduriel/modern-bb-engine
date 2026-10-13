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
	g: i32,
	/// The cost of stepping onto this node's tile (it depends only on the tile).
	step: i32,
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

/// `trunc(cost * (road ? RoadMult : 1.0) + heat)` in `f32`, as the original computes a
/// step's cost (`cost` already `* 10`, `heat` already `* HeatCost`).
fn step_cost(cost: i32, road: bool, heat: i32, settings: &NavSettings) -> i32 {
	let mult = if road { settings.road_mult } else { 1.0 };
	cvtt_i32(f64::from(cost as f32 * mult + heat as f32))
}

fn hex_distance(aq: i64, ar: i64, bq: i64, br: i64) -> i64 {
	let (dq, dr) = (aq - bq, ar - br);
	(dq.abs() + dr.abs() + (dq + dr).abs()) / 2
}

/// Where a tile stands in the search. The original keeps separate open and closed sets
/// but checks closed first, so "closed" wins whenever a tile would be in both; one state
/// per tile is therefore enough. Under [`find_path`]'s contract a tile is never in both
/// anyway: it leaves the open set when it is closed, and a tile that passed the
/// impassable checks once passes them again.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TileState {
	Unseen,
	/// On the open list, as this node.
	Open(usize),
	Closed,
}

/// The per-tile states, indexed by tile id (the grid is dense and small), growing to
/// the largest id the search touches. Packed into a `u32` each to keep the array
/// cache-resident: the search's generation in the top 8 bits (an entry from an older
/// search reads as unseen, so the array is reused without clearing it), then 0 for
/// closed or node index + 1 for open.
#[derive(Default)]
struct TileStates {
	states: Vec<u32>,
	generation: u32,
}

const GENERATION_SHIFT: u32 = 24;
const NODE_MASK: u32 = (1 << GENERATION_SHIFT) - 1;

impl TileStates {
	/// Forget every state (start a new search).
	fn reset(&mut self) {
		self.generation += 1;
		if self.generation > u32::MAX >> GENERATION_SHIFT {
			self.states.fill(0);
			self.generation = 1;
		}
	}

	fn get(&self, id: usize) -> TileState {
		match self.states.get(id).copied() {
			Some(v) if v >> GENERATION_SHIFT == self.generation => match v & NODE_MASK {
				0 => TileState::Closed,
				n => TileState::Open(n as usize - 1),
			},
			_ => TileState::Unseen,
		}
	}

	fn set(&mut self, id: usize, state: TileState) {
		if id >= self.states.len() {
			self.states.resize(id + 1, 0);
		}
		let value = match state {
			TileState::Unseen => unreachable!("states are only ever advanced"),
			TileState::Closed => 0,
			TileState::Open(node) => u32::try_from(node + 1)
				.ok()
				.filter(|&n| n <= NODE_MASK)
				.expect("node count fits in 24 bits"),
		};
		self.states[id] = self.generation << GENERATION_SHIFT | value;
	}
}

/// An open-list entry: a node and its `f`, kept inline so heap moves and comparisons
/// stay inside the heap array.
#[derive(Clone, Copy)]
struct Entry {
	f: i32,
	node: u32,
}

/// The open list: MSVC `std::` heap primitives with the original's comparator
/// `less(a, b) = a.f > b.f` (a min-heap on `f`, no secondary key).
#[derive(Default)]
struct OpenList(Vec<Entry>);

fn less(a: Entry, b: Entry) -> bool {
	a.f > b.f
}

impl OpenList {
	fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	fn push_by_index(h: &mut [Entry], mut hole: usize, top: usize, val: Entry) {
		while top < hole {
			let idx = (hole - 1) / 2;
			if !less(h[idx], val) {
				break;
			}
			h[hole] = h[idx];
			hole = idx;
		}
		h[hole] = val;
	}

	fn adjust(h: &mut [Entry], mut hole: usize, bottom: usize, val: Entry) {
		let top = hole;
		let mut idx = hole;
		// `bottom >= 1` at every call site (pop of >= 2 elements, make of >= 2).
		let max_non_leaf = (bottom - 1) / 2;
		// Two levels per iteration while both children would descend again: the child
		// and both candidate grandchild pairs are compared independently, so the next
		// position waits on one load instead of two (same path, same moves).
		while 2 * idx + 2 < max_non_leaf {
			let right = 2 * idx + 2;
			let left_wins = less(h[right], h[right - 1]);
			let child = right - usize::from(left_wins);
			let (ll, lr) = (2 * right - 1, 2 * right);
			let (rl, rr) = (2 * right + 1, 2 * right + 2);
			let gl = lr - usize::from(less(h[lr], h[ll]));
			let gr = rr - usize::from(less(h[rr], h[rl]));
			let mask = 0usize.wrapping_sub(usize::from(left_wins));
			let grandchild = gr ^ ((gr ^ gl) & mask);
			h[hole] = h[child];
			h[child] = h[grandchild];
			hole = grandchild;
			idx = grandchild;
		}
		while idx < max_non_leaf {
			idx = 2 * idx + 2;
			// Branch-free: which child wins is data-dependent and mispredicts often.
			idx -= usize::from(less(h[idx], h[idx - 1]));
			h[hole] = h[idx];
			hole = idx;
		}
		if idx == max_non_leaf && bottom.is_multiple_of(2) {
			h[hole] = h[bottom - 1];
			hole = bottom - 1;
		}
		Self::push_by_index(h, hole, top, val);
	}

	/// MSVC `make_heap`: `adjust(i, n, h[i])` for every internal node, bottom-up.
	///
	/// Each step is computed as a sift-down that stops at the first child with a larger
	/// `f`, which gives exactly the array `adjust`'s sift-to-leaf-then-push-back does:
	/// bottom-up, both subtrees of `i` are already heaps, so the `f`s along `adjust`'s
	/// descent path never decrease; the push back up therefore stops right below the
	/// deepest path element whose `f` is `<= val.f`, i.e. at the end of the prefix this
	/// loop walks (the path is the same: same child choice, ties to the right). Usually
	/// that prefix is empty (`val` already beats its children), so the rebuild costs one
	/// comparison per internal node instead of a walk to a leaf and back.
	fn make(h: &mut [Entry]) {
		let n = h.len();
		let mut i = n / 2;
		while i > 0 {
			i -= 1;
			let val = h[i];
			let mut hole = i;
			loop {
				let right = 2 * hole + 2;
				let child = if right < n {
					right - usize::from(less(h[right], h[right - 1]))
				} else if right == n {
					right - 1
				} else {
					break;
				};
				if less(h[child], val) {
					break;
				}
				h[hole] = h[child];
				hole = child;
			}
			h[hole] = val;
		}
	}

	fn push(&mut self, node: usize, f: i32) {
		let val = Entry {
			f,
			node: u32::try_from(node).expect("node count fits in u32"),
		};
		self.0.push(val);
		let last = self.0.len() - 1;
		Self::push_by_index(&mut self.0, last, 0, val);
	}

	fn pop(&mut self) -> usize {
		let h = &mut self.0;
		let n = h.len();
		if n >= 2 {
			let val = h[n - 1];
			h[n - 1] = h[0];
			Self::adjust(h, 0, n - 1, val);
		}
		h.pop().expect("pop on a non-empty heap").node as usize
	}

	/// Lower `node`'s `f`, then rebuild the whole heap as the original does (`make_heap`
	/// after the cost update).
	fn lower(&mut self, node: usize, f: i32) {
		let e = self
			.0
			.iter_mut()
			.find(|e| e.node as usize == node)
			.expect("lowered node is on the open list");
		e.f = f;
		Self::make(&mut self.0);
	}
}

/// The path from axial `start` to `goal`: the waypoint tile ids after the start, ending
/// at the goal (or, with `max_len`/`StopAtRoad`, where the search stopped). Empty if
/// there is none. `tile(q, r)` resolves an in-grid axial coordinate.
///
/// Contract: `tile` gives every in-grid coordinate its own id (the grid's flat index),
/// and a tile's data does not change during the search. The search keys its state and
/// cached step costs by id, so a lookup that broke either rule could diverge from the
/// original (which re-reads every tile each time it is generated).
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

	// The step cost into a tile without heat depends only on its type and road flag.
	let no_heat_steps: [[i32; 2]; 32] = std::array::from_fn(|t| {
		let cost = settings.costs[t].wrapping_mul(10);
		[
			step_cost(cost, false, 0, settings),
			step_cost(cost, true, 0, settings),
		]
	});

	// Reuse the previous search's buffers (taken out of the slot, so a re-entrant call
	// would just start with empty ones).
	let mut scratch = SCRATCH.take();
	scratch.reset();
	let path = search(
		&mut scratch,
		start,
		goal,
		start_tile.id,
		goal_tile.id,
		max_len,
		settings,
		&no_heat_steps,
		tile,
	);
	SCRATCH.set(scratch);
	path
}

/// The search's buffers, kept between calls.
#[derive(Default)]
struct Scratch {
	nodes: Vec<Node>,
	states: TileStates,
	open: OpenList,
}

impl Scratch {
	fn reset(&mut self) {
		self.nodes.clear();
		self.states.reset();
		self.open.0.clear();
	}
}

thread_local! {
	static SCRATCH: std::cell::Cell<Scratch> = std::cell::Cell::new(Scratch::default());
}

#[allow(clippy::too_many_arguments)]
fn search(
	scratch: &mut Scratch,
	start: (i64, i64),
	goal: (i64, i64),
	start_id: usize,
	goal_id: usize,
	max_len: i64,
	settings: &NavSettings,
	no_heat_steps: &[[i32; 2]; 32],
	tile: impl Fn(i64, i64) -> Option<NavTile>,
) -> Vec<usize> {
	let Scratch {
		nodes,
		states,
		open,
	} = scratch;
	nodes.push(Node {
		g: 0,
		step: 0,
		q: start.0,
		r: start.1,
		id: start_id,
		parent: None,
		dir: NO_DIR,
	});
	states.set(start_id, TileState::Open(0));
	open.push(0, 0);

	while !open.is_empty() {
		let cur = open.pop();
		let (cq, cr, cdir, cg) = {
			let c = &nodes[cur];
			(c.q, c.r, c.dir, c.g)
		};
		// By reference: iterating the array by value copies it (a `memcpy` call) per node.
		for &(dq, dr, dir, skip) in &BLOCKS {
			if cdir == skip {
				continue;
			}
			let (nq, nr) = (cq + dq, cr + dr);
			let Some(nt) = tile(nq, nr) else {
				continue;
			};
			if nt.id == goal_id || (settings.stop_at_road && nt.road) {
				return reconstruct(nodes, cur, goal_id);
			}
			let idx = match states.get(nt.id) {
				TileState::Closed => continue,
				TileState::Open(idx) => idx,
				TileState::Unseen => {
					let cost = settings
						.costs
						.get(nt.type_)
						.map_or(0, |c| c.wrapping_mul(10));
					if cost == 0 || (settings.road_only && !nt.road) {
						states.set(nt.id, TileState::Closed);
						continue;
					}
					let dist = hex_distance(nq, nr, goal.0, goal.1);
					if max_len != 0 && dist <= max_len && dist != 1 {
						return reconstruct(nodes, cur, nt.id);
					}
					let heat = i32::from(nt.heat).wrapping_mul(settings.heat_cost);
					let step = if heat == 0 {
						// `type_ < 32` here: other types have no cost and were closed above.
						no_heat_steps[nt.type_][usize::from(nt.road)]
					} else {
						step_cost(cost, nt.road, heat, settings)
					};
					let g = cg.wrapping_add(step);
					let idx = nodes.len();
					nodes.push(Node {
						g,
						step,
						q: nq,
						r: nr,
						id: nt.id,
						parent: Some(cur),
						dir,
					});
					states.set(nt.id, TileState::Open(idx));
					open.push(idx, g.wrapping_add(20 * dist as i32));
					continue;
				}
			};
			// An open tile passed the cost and `RoadOnly` checks when it was opened, and its
			// step cost (which depends only on the tile) is cached on its node. The `max_len`
			// check mirrors the original; under the contract it cannot fire here (the tile's
			// distance was checked the same way when it was opened).
			let dist = || hex_distance(nq, nr, goal.0, goal.1);
			if max_len != 0 && dist() <= max_len && dist() != 1 {
				return reconstruct(nodes, cur, nt.id);
			}
			let n = &mut nodes[idx];
			let g = cg.wrapping_add(n.step);
			if g < n.g {
				n.g = g;
				n.parent = Some(cur);
				n.dir = dir;
				open.lower(idx, g.wrapping_add(20 * dist() as i32));
			}
		}
		states.set(nodes[cur].id, TileState::Closed);
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
