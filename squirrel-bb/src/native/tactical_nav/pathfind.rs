//! `navigator.findPath(from, to, settings, within)`: the original's A* (its
//! `TacticalNavigator::findPath`), reproduced rule for rule so paths, their costs and
//! their tie-breaks match.
//!
//! Costs are in tenths of an action point. Stepping onto a tile costs
//! `10·ActionPointCosts[type] + trunc(10·FatigueCosts[type]·FatigueCostFactor)`, plus
//! `|Δlevel|` times the per-level cost, plus `HeightLevelCost·level`,
//! `max(heat·HeatCost, 0)`, `HiddenCost` on hiding terrain, minus `TVTotal/6`. Then,
//! outside the goal test: +2 to pass an ally standing outside hostile zones of control,
//! `TVLevelDisadvantage` for the AI, `10·ZoneOfControlCost` for an empty tile in a hostile
//! zone. The estimate adds `20·distance`.
//!
//! Search order: neighbours in the direction order 0, 3, 1, 5, 4, 2 (never straight
//! back), open nodes on an MSVC binary heap ([`heap`](super::heap)), costs improved in
//! place with a full re-heapify. Reaching a tile within `within` of the target (not
//! occupied, unless it's the start or `TileToConsiderEmpty`) records the goal and ends
//! that node's expansion; the search then pops at most three more nodes looking for a
//! cheaper way in. An allied unit can be passed through, but not two in a row; other
//! units and objects (faction 0) block. With `AllowZoneOfControlPassing` off, empty
//! tiles in hostile zones are closed outright.

use super::heap;
use super::settings::NavSettings;
use crate::native::tactical_grid::{TacticalGridState, TacticalTileData, hex_distance};

/// One step of a found path: the tile entered and the direction moved to enter it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Step {
	pub tile: usize,
	pub dir: i32,
}

#[derive(Clone, Copy, Default)]
struct Node {
	/// Estimate (cost + 20·distance); the goal node's is its cost.
	f: i32,
	g: i32,
	level: i32,
	q: i64,
	r: i64,
	parent: Option<usize>,
	/// The direction entered by; 6 for the start.
	dir: i32,
	/// Whether a unit stands on it (one can't pass two in a row).
	on_entity: bool,
}

/// A search's working memory, kept between searches so a search doesn't allocate and
/// clear a node per map tile (an AI turn runs dozens). A node is only read once the
/// search has written it; `open` and `closed` hold the number of the search that marked
/// the tile, so starting a search clears them all at once.
#[derive(Default)]
struct Scratch {
	nodes: Vec<Node>,
	open: Vec<u32>,
	closed: Vec<u32>,
	heap: Vec<usize>,
	search: u32,
}

impl Scratch {
	/// Ready for a search over `n` tiles: every tile neither open nor closed.
	fn begin(&mut self, n: usize) {
		if self.nodes.len() < n {
			self.nodes.resize(n, Node::default());
			self.open.resize(n, 0);
			self.closed.resize(n, 0);
		}
		self.search = self.search.wrapping_add(1);
		if self.search == 0 {
			self.open.fill(0);
			self.closed.fill(0);
			self.search = 1;
		}
		self.heap.clear();
	}
}

thread_local! {
	static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::default();
}

/// Directions in the order the original expands them, with their axial steps.
const EXPANSION: [(i32, i64, i64); 6] = [(0, 0, 1), (3, 0, -1), (1, 1, 0), (5, -1, 1), (4, -1, 0), (2, 1, -1)];

/// What a search reads: the map, the faction of whoever stands on each occupied tile,
/// and the settings.
pub(crate) struct Query<'a> {
	pub grid: &'a TacticalGridState,
	/// The faction of the entity on tile `i` (`None` when the tile is empty).
	pub occupant: &'a [Option<i64>],
	pub settings: &'a NavSettings,
}

impl Query<'_> {
	fn tile(&self, i: usize) -> &TacticalTileData {
		&self.grid.tiles[i]
	}

	/// Whether faction `f`'s unit can be walked through (allied and not faction 0).
	fn passable_unit(&self, f: i64) -> bool {
		f != 0 && !self.settings.is_hostile(f)
	}

	fn in_hostile_zone(&self, t: &TacticalTileData) -> bool {
		self.settings.hostile_zone(&t.zoc)
	}
}

/// The per-terrain step costs and the per-level cost, in tenths of an AP.
fn step_costs(s: &NavSettings) -> ([i32; 16], i32) {
	let weigh = |ap: i32, fat: i32| ((fat.wrapping_mul(10)) as f32 * s.fatigue_weight) as i32 + ap.wrapping_mul(10);
	let mut costs = [0; 16];
	for (i, c) in costs.iter_mut().enumerate() {
		*c = weigh(s.ap_costs[i], s.fatigue_costs[i]);
	}
	(costs, weigh(s.ap_per_level, s.fatigue_per_level))
}

/// Whether some neighbour of the target leads into it without crossing a hostile zone
/// (the original's pre-check when the target itself must be reached with zones
/// blocking).
fn target_enterable(q: &Query, start: usize, dest: usize) -> bool {
	let s = q.settings;
	let d = q.tile(dest);
	for (_, dq, dr) in [(0, 0, 1), (1, 1, 0), (2, 1, -1), (3, 0, -1), (4, -1, 0), (5, -1, 1)] {
		let Some(n) = q.grid.idx_checked(d.q + dq, d.r + dr) else { continue };
		let nt = q.tile(n);
		if (d.level - nt.level).abs() as i32 > s.max_level_difference {
			continue;
		}
		if n == start {
			return true;
		}
		match q.occupant[n] {
			Some(f) => {
				if !s.is_hostile(f) {
					return true;
				}
			}
			None => {
				if !q.in_hostile_zone(nt) {
					return true;
				}
			}
		}
	}
	false
}

/// Search from `start` to within `within` tiles of `dest`. Returns the steps (the
/// first move first, the goal last; the start excluded), or `None` if there is no way.
pub(crate) fn find_path(q: &Query, start: usize, dest: usize, within: i32) -> Option<Vec<Step>> {
	// A search runs no script, so it never re-enters.
	SCRATCH.with_borrow_mut(|scratch| search(q, start, dest, within, scratch))
}

fn search(q: &Query, start: usize, dest: usize, within: i32, scratch: &mut Scratch) -> Option<Vec<Step>> {
	let s = q.settings;
	if start == dest {
		return None;
	}
	if within == 0 {
		let d = q.tile(dest);
		if q.occupant[dest].is_some() || d.type_ == 0 {
			return None;
		}
		if !s.allow_zoc_passing && !target_enterable(q, start, dest) {
			return None;
		}
	}
	let (costs, level_cost) = step_costs(s);
	let n_tiles = q.grid.tiles.len();
	scratch.begin(n_tiles);
	let Scratch { nodes, open, closed, heap, search: this } = scratch;
	let this = *this;
	let dest_tile = q.tile(dest);

	let st = q.tile(start);
	nodes[start] = Node {
		f: 0,
		g: 0,
		level: st.level as i32,
		q: st.q,
		r: st.r,
		parent: None,
		dir: 6,
		on_entity: false,
	};
	open[start] = this;
	heap.push(start);
	let mut found = false;
	let mut extra_pops = 3;

	while !heap.is_empty() {
		if found {
			let left = extra_pops;
			extra_pops -= 1;
			if left == 0 {
				break;
			}
		}
		let cur = {
			let less = |a: usize, b: usize| nodes[a].f > nodes[b].f;
			heap::pop(heap, &less).expect("heap is non-empty")
		};
		let c = nodes[cur];

		for (dir, dq, dr) in EXPANSION {
			if c.dir == (dir + 3) % 6 {
				continue;
			}
			let Some(ti) = q.grid.idx_checked(c.q + dq, c.r + dr) else { continue };
			if closed[ti] == this {
				continue;
			}
			let t = q.tile(ti);
			if let Some(f) = q.occupant[ti] {
				if Some(ti) != s.consider_empty && (!q.passable_unit(f) || c.on_entity) {
					continue;
				}
			}
			if s.is_player && !t.is_discovered {
				continue;
			}
			let level = t.level as i32;
			let climb = (level - c.level).abs();
			if climb > s.max_level_difference {
				continue;
			}
			let terrain = costs.get(t.type_ as usize).copied().unwrap_or(0);
			if terrain == 0 {
				continue;
			}
			let dist = hex_distance(t, dest_tile) as i32;
			let heat = (t.heat.wrapping_mul(s.heat_cost)).max(0);
			let hidden = if t.is_hiding_entity { s.hidden_cost } else { 0 };
			let tile_cost = terrain
				.wrapping_add(heat)
				.wrapping_add(s.height_level_cost.wrapping_mul(level))
				.wrapping_sub(t.tv_total as i32 / 6);
			let mut g = climb.wrapping_mul(level_cost).wrapping_add(tile_cost).wrapping_add(c.g).wrapping_add(hidden);
			let occupied = q.occupant[ti].is_some();

			// The goal test: within reach of the target (adjacent ones also within the
			// level limit of it), and free to stand on.
			let reach_ok = dist <= within && !(dist == 1 && (level - dest_tile.level as i32).abs() > s.max_level_difference);
			let standable = !occupied || ti == start || Some(ti) == s.consider_empty;
			if reach_ok && standable {
				if found {
					if g < nodes[dest].g {
						let n = &mut nodes[dest];
						n.f = g;
						n.g = g;
						n.dir = dir;
						n.parent = Some(cur);
					}
				} else {
					nodes[dest] = Node {
						f: g,
						g,
						level,
						q: t.q,
						r: t.r,
						parent: Some(cur),
						dir,
						on_entity: occupied,
					};
				}
				found = true;
				break;
			}

			let hostile_zone = q.in_hostile_zone(t);
			if !s.allow_zoc_passing && hostile_zone && !occupied {
				closed[ti] = this;
				continue;
			}
			if occupied && !hostile_zone {
				g = g.wrapping_add(2);
			}
			if !s.is_player {
				g = g.wrapping_add(t.tv_level_disadvantage as i32);
			}
			if hostile_zone && !occupied {
				g = g.wrapping_add(s.zoc_cost.wrapping_mul(10));
			}
			let f = g.wrapping_add(dist.wrapping_mul(20));
			if open[ti] != this {
				nodes[ti] = Node {
					f,
					g,
					level,
					q: t.q,
					r: t.r,
					parent: Some(cur),
					dir,
					on_entity: occupied,
				};
				let less = |a: usize, b: usize| nodes[a].f > nodes[b].f;
				heap::push(heap, ti, &less);
				open[ti] = this;
			} else if g < nodes[ti].g {
				let n = &mut nodes[ti];
				n.g = g;
				n.parent = Some(cur);
				n.dir = dir;
				n.f = f;
				let less = |a: usize, b: usize| nodes[a].f > nodes[b].f;
				heap::make(heap, &less);
			}
		}
		closed[cur] = this;
		open[cur] = 0;
	}

	if !found {
		return None;
	}
	let mut steps = Vec::new();
	let mut n = nodes[dest];
	while let Some(parent) = n.parent {
		// A chain can't be longer than the map (guards against a corrupt one).
		if steps.len() > n_tiles {
			break;
		}
		let tile = q.grid.idx_checked(n.q, n.r).expect("path node is on the map");
		steps.push(Step { tile, dir: n.dir });
		n = nodes[parent];
	}
	steps.reverse();
	Some(steps)
}

/// `clipPathToDistance(tile, d)`: keep the leading steps within `d` of `tile`, cutting
/// from the first one beyond.
pub(crate) fn clip_to_distance(grid: &TacticalGridState, path: &mut Vec<Step>, tile: usize, d: i64) {
	if let Some(i) = path.iter().position(|s| hex_distance(&grid.tiles[s.tile], &grid.tiles[tile]) > d) {
		path.truncate(i);
	}
}
