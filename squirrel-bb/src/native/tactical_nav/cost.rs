//! What a found path costs a unit: `getCostForPath` and `getTurnsRequiredForPath`,
//! as the original computes them (whole action points and fatigue, not the search's
//! tenths).

use super::pathfind::Step;
use super::settings::NavSettings;
use crate::native::tactical_grid::{TacticalGridState, hex_distance};

/// `getCostForPath`'s result.
pub(crate) struct PathCost {
	/// AP / fatigue spent on the reachable part.
	pub ap: i32,
	pub fatigue: i32,
	/// AP / fatigue the whole path needs.
	pub ap_required: i32,
	pub fatigue_required: i32,
	/// Steps reachable.
	pub tiles: i32,
	pub is_complete: bool,
	pub missing_ap: bool,
	pub missing_fatigue: bool,
	pub first: usize,
	/// Where the reachable part ends, and the tiles before it.
	pub end: usize,
	pub last_before_end: usize,
	pub second_last_before_end: usize,
}

fn type_cost(costs: &[i32; 16], ty: i64) -> i32 {
	costs.get(ty as usize).copied().unwrap_or(0)
}

/// Walk `path` from `from` (the unit's tile) with `ap`/`fatigue` to spend.
///
/// Steps stay reachable until one would overspend either; after that only the
/// "required" totals grow (and the missing flags can still be raised against the
/// totals of the reachable part). If the reachable part ends on an occupied tile it
/// is cut back one step, leaving `end == last_before_end`.
pub(crate) fn cost_for_path(
	grid: &TacticalGridState,
	occupied: impl Fn(usize) -> bool,
	path: &[Step],
	s: &NavSettings,
	from: usize,
	ap_max: i32,
	fatigue_max: i32,
) -> PathCost {
	let mut level = grid.tiles[from].level as i32;
	let mut r = PathCost {
		ap: 0,
		fatigue: 0,
		ap_required: 0,
		fatigue_required: 0,
		tiles: 0,
		is_complete: false,
		missing_ap: false,
		missing_fatigue: false,
		first: path.first().map_or(from, |st| st.tile),
		end: from,
		last_before_end: from,
		second_last_before_end: from,
	};
	let (mut prev_ap, mut prev_fatigue) = (0, 0);
	let mut stopped = false;
	for step in path {
		let t = &grid.tiles[step.tile];
		let climb = (level - t.level as i32).abs();
		let ap_cost = climb.wrapping_mul(s.ap_per_level).wrapping_add(type_cost(&s.ap_costs, t.type_));
		let ap = r.ap.wrapping_add(ap_cost);
		if ap > ap_max {
			r.missing_ap = true;
			stopped = true;
		}
		let fatigue_cost = climb.wrapping_mul(s.fatigue_per_level).wrapping_add(type_cost(&s.fatigue_costs, t.type_));
		let fatigue = r.fatigue.wrapping_add(fatigue_cost);
		if fatigue > fatigue_max {
			r.missing_fatigue = true;
			stopped = true;
		}
		r.ap_required = r.ap_required.wrapping_add(ap_cost);
		r.fatigue_required = r.fatigue_required.wrapping_add(fatigue_cost);
		if !stopped {
			r.second_last_before_end = r.last_before_end;
			prev_fatigue = r.fatigue;
			prev_ap = r.ap;
			r.last_before_end = r.end;
			r.tiles += 1;
			r.fatigue = fatigue;
			r.ap = ap;
			r.end = step.tile;
		}
		level = t.level as i32;
	}
	if r.end != from && occupied(r.end) {
		r.ap = prev_ap;
		r.tiles -= 1;
		r.end = r.last_before_end;
		r.fatigue = prev_fatigue;
	}
	r.is_complete = r.tiles as usize == path.len();
	r
}

/// `getTurnsRequiredForPath(unit, settings, ap [, target, faction])`: the path's AP
/// cost over `ap` (an f32 ratio, as the original returns).
///
/// Every step but the last adds `ZoneOfControlCost` when it lies in a hostile zone of
/// control. With a `target` tile and a hostile `faction`, one of the zones next to the
/// target is assumed to be that faction's own and discounted.
pub(crate) fn turns_required(
	grid: &TacticalGridState,
	path: &[Step],
	s: &NavSettings,
	from: usize,
	ap: i32,
	target: Option<(usize, i64)>,
) -> f32 {
	let mut level = grid.tiles[from].level as i32;
	let mut cost: i32 = 0;
	let last = path.len().wrapping_sub(1);
	for (i, step) in path.iter().enumerate() {
		let t = &grid.tiles[step.tile];
		let climb = (level - t.level as i32).abs();
		cost = cost.wrapping_add(climb.wrapping_mul(s.ap_per_level).wrapping_add(type_cost(&s.ap_costs, t.type_)));
		if i != last {
			let in_zone = match target {
				None => s.hostile_zone(&t.zoc),
				Some((target, faction)) => {
					let mut count = s.hostile_zone_count(&t.zoc);
					if faction != 0
						&& s.is_hostile(faction)
						&& hex_distance(&grid.tiles[target], t) == 1
					{
						count -= 1;
					}
					count > 0
				}
			};
			if in_zone {
				cost = cost.wrapping_add(s.zoc_cost);
			}
		}
		level = t.level as i32;
	}
	cost as f32 / ap as f32
}
