//! `navigator.travel(unit, ap, fatigue)`: moving a unit along the found path, as the
//! original does it (disassembled).
//!
//! The first call settles the whole move up front. Step by step it runs the unit's
//! `onMovementStep(tile, levelDifference)` (which spends its AP and fatigue and
//! updates its vision), and cuts the path short where:
//!
//! - the first step can't be paid: no move;
//! - leaving the start, a neighbour gets a free attack (`onAttackOfOpportunity`): no
//!   move;
//! - leaving a later empty tile, a neighbour would attack (`onMovementInZoneOfControl`):
//!   stop there;
//! - a step can't be paid: stop before it (passing an ally, give that step back with
//!   `onMovementUndo` and stop one earlier);
//! - entering an empty tile, a neighbour would attack: stop there, and take the free
//!   attacks on arrival.
//!
//! Then `onMovementStart(tile, steps)`. A move the player can't see (neither the start
//! nor the destination is in view) completes at once and `travel` answers false. A
//! visible one animates: the unit takes its destination tile once it lifts off and
//! `onMovementFinish` runs when it lands, and `travel` answers true until then. The
//! scripts call it once per frame while it does. Headless, each animation phase (lift
//! off, move, land) takes one call; the original times them (0.14 s, 0.18 s per tile,
//! 0.14 s, faster with the faster-movement settings), so only frame timing differs.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef};

use super::NavState;
use crate::native::tactical_entities::{Units, unit_identity};
use crate::native::tactical_grid::{self, Grid, zone::faction_bit};

/// The player's faction (whose view decides whether a move animates).
const PLAYER: i64 = 1;
/// The six neighbour steps, in the original's scan order.
const STEP: [(i64, i64); 6] = [(0, 1), (1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1)];

/// An animated move in progress.
pub(crate) struct Travel {
	pub identity: i64,
	entity: SqStrongRef,
	dest: usize,
	/// The last empty tile reached (where a unit attacked on arrival resettles).
	last_empty: usize,
	attacked_on_arrival: bool,
	phase: Phase,
}

#[derive(Clone, Copy)]
enum Phase {
	LiftOff,
	Moving,
	Landing,
}

fn tile(g: &Grid, vm: &SQVM, id: usize) -> Result<SqStrongRef, String> {
	tactical_grid::tile_handle(g, vm, id).ok_or_else(|| "travel: no tile class".to_string())
}

fn seen_by_player(g: &Grid, id: usize) -> bool {
	g.borrow().tiles[id].vis_mask & faction_bit(PLAYER) != 0
}

/// The units next to tile `id` (within a level) that a mover there must reckon with,
/// in the original's scan order; none if no zone of control reaches the tile.
fn neighbours_in_zone(g: &Grid, id: usize, mover: i64) -> Vec<SqStrongRef> {
	let gb = g.borrow();
	let t = &gb.tiles[id];
	if t.zoc.mask == 0 {
		return Vec::new();
	}
	STEP.iter()
		.filter_map(|(dq, dr)| gb.idx_checked(t.q + dq, t.r + dr))
		.filter(|&n| (gb.tiles[n].level - t.level).abs() <= 1)
		.filter_map(|n| gb.tiles[n].occupant.as_ref().map(|(e, _)| e.clone()))
		.filter(|e| e.identity() != mover)
		.collect()
}

/// Ask the neighbours `method(mover, on_enter)` in turn; the first `true` answers.
fn any_neighbour(vm: &SQVM, g: &Grid, id: usize, mover: &SqStrongRef, on_enter: bool, method: &str) -> Result<bool, String> {
	for e in neighbours_in_zone(g, id, mover.identity()) {
		match e.call_method_ret::<bool, _>(vm, method, (mover.clone(), on_enter)) {
			Ok(Some(true)) => return Ok(true),
			Ok(_) => {}
			// A neighbour without the hook (a map object) doesn't take part.
			Err(e) if e.starts_with("no method") => {}
			Err(e) => return Err(format!("{method}: {e}")),
		}
	}
	Ok(false)
}

/// Whether the mover may stay or move on here without a neighbour stopping it.
fn zone_lets_pass(vm: &SQVM, g: &Grid, id: usize, mover: &SqStrongRef, on_enter: bool) -> Result<bool, String> {
	Ok(!any_neighbour(vm, g, id, mover, on_enter, "onMovementInZoneOfControl")?)
}

/// The neighbours' free attacks on the mover at `id`; whether one happened.
fn free_attack(vm: &SQVM, g: &Grid, id: usize, mover: &SqStrongRef, on_enter: bool) -> Result<bool, String> {
	any_neighbour(vm, g, id, mover, on_enter, "onAttackOfOpportunity")
}

fn step_ok(vm: &SQVM, mover: &SqStrongRef, method: &str, t: SqStrongRef, diff: i64) -> Result<bool, String> {
	Ok(mover.call_method_ret::<bool, _>(vm, method, (t, diff))?.unwrap_or(false))
}

/// A hook's result, where a unit without the hook is fine.
fn hook(method: &str, result: Result<(), String>) -> Result<(), String> {
	match result {
		Err(e) if e.starts_with("no method") => Ok(()),
		other => other.map_err(|e| format!("{method}: {e}")),
	}
}

fn movement_start(vm: &SQVM, mover: &SqStrongRef, t: SqStrongRef, steps: i64) -> Result<(), String> {
	hook("onMovementStart", mover.call_method(vm, "onMovementStart", (t, steps)))
}

fn movement_finish(vm: &SQVM, mover: &SqStrongRef, t: SqStrongRef) -> Result<(), String> {
	hook("onMovementFinish", mover.call_method(vm, "onMovementFinish", (t,)))
}

fn is_alive(vm: &SQVM, mover: &SqStrongRef) -> bool {
	mover.call_method_ret::<bool, _>(vm, "isAlive", ()).ok().flatten().unwrap_or(true)
}

/// `travel(entity, ...)`.
pub(super) fn travel(nav: &Rc<RefCell<NavState>>, vm: &SQVM, entity: &SqStrongRef) -> Result<bool, String> {
	if nav.borrow().travel.is_some() {
		return advance(nav, vm);
	}
	let (grid, units, path) = {
		let st = nav.borrow();
		(st.grid.clone(), st.units.clone(), st.path.clone())
	};
	if path.is_empty() {
		return Ok(false);
	}
	let identity = unit_identity(vm, entity);
	let (Some(mover), Some(start)) = (units.entity(identity), units.info(identity).and_then(|u| u.tile)) else {
		return Ok(false);
	};
	let occupied = |id: usize| grid.borrow().tiles[id].occupant.is_some();
	let level = |id: usize| grid.borrow().tiles[id].level;

	// Settle the move.
	let mut keep = path.len();
	let mut cur = start;
	let mut cur_level = level(start);
	let mut prev_level = cur_level;
	let mut last_empty = start;
	let mut attacked_on_arrival = false;
	for (i, step) in path.iter().enumerate() {
		let t = step.tile;
		if cur == start {
			if !step_ok(vm, &mover, "onMovementStep", tile(&grid, vm, t)?, level(t) - cur_level)? {
				keep = 0;
				break;
			}
			if free_attack(vm, &grid, start, &mover, false)? {
				keep = 0;
				break;
			}
		} else {
			if !occupied(cur) && !zone_lets_pass(vm, &grid, cur, &mover, false)? {
				keep = i;
				break;
			}
			if !step_ok(vm, &mover, "onMovementStep", tile(&grid, vm, t)?, level(t) - cur_level)? {
				if occupied(cur) {
					step_ok(vm, &mover, "onMovementUndo", tile(&grid, vm, cur)?, cur_level - prev_level)?;
					keep = i - 1;
				} else {
					keep = i;
				}
				break;
			}
		}
		if !occupied(t) && !zone_lets_pass(vm, &grid, t, &mover, true)? {
			attacked_on_arrival = true;
			keep = i + 1;
			break;
		}
		prev_level = cur_level;
		cur_level = level(t);
		cur = t;
		if !occupied(t) {
			last_empty = t;
		}
	}
	let path: Vec<_> = path.into_iter().take(keep).collect();
	nav.borrow_mut().path = path.clone();
	let Some(dest) = path.last().map(|s| s.tile) else {
		return Ok(false);
	};

	movement_start(vm, &mover, tile(&grid, vm, start)?, path.len() as i64)?;
	if !seen_by_player(&grid, start) && !seen_by_player(&grid, dest) {
		// Out of the player's sight: done at once.
		units.move_to(identity, dest)?;
		movement_finish(vm, &mover, tile(&grid, vm, dest)?)?;
		if attacked_on_arrival {
			resettle_after_free_attack(vm, &grid, &units, identity, &mover, last_empty)?;
		}
		nav.borrow_mut().path.clear();
		return Ok(false);
	}
	if seen_by_player(&grid, dest) {
		units.discover(vm, identity)?;
	}
	nav.borrow_mut().travel = Some(Travel {
		identity,
		entity: mover,
		dest,
		last_empty,
		attacked_on_arrival,
		phase: Phase::LiftOff,
	});
	Ok(true)
}

/// A unit that took free attacks on arriving starts and finishes a move again where it
/// stands (the original then re-settles it on the last empty tile it reached, which
/// is where it stands).
fn resettle_after_free_attack(
	vm: &SQVM,
	grid: &Grid,
	units: &Units,
	identity: i64,
	mover: &SqStrongRef,
	last_empty: usize,
) -> Result<(), String> {
	let Some(here) = units.info(identity).and_then(|u| u.tile) else { return Ok(()) };
	if free_attack(vm, grid, here, mover, true)? && is_alive(vm, mover) {
		movement_start(vm, mover, tile(grid, vm, here)?, 1)?;
		units.move_to(identity, last_empty)?;
		let now = units.info(identity).and_then(|u| u.tile).unwrap_or(here);
		movement_finish(vm, mover, tile(grid, vm, now)?)?;
	}
	Ok(())
}

/// One later `travel` call: the next animation phase.
fn advance(nav: &Rc<RefCell<NavState>>, vm: &SQVM) -> Result<bool, String> {
	let (grid, units, phase, identity, dest) = {
		let st = nav.borrow();
		let t = st.travel.as_ref().expect("travelling");
		(st.grid.clone(), st.units.clone(), t.phase, t.identity, t.dest)
	};
	match phase {
		Phase::LiftOff => {
			units.move_to(identity, dest)?;
			nav.borrow_mut().travel.as_mut().expect("travelling").phase = Phase::Moving;
			Ok(true)
		}
		Phase::Moving => {
			if seen_by_player(&grid, dest) {
				units.discover(vm, identity)?;
			}
			nav.borrow_mut().travel.as_mut().expect("travelling").phase = Phase::Landing;
			Ok(true)
		}
		Phase::Landing => {
			let t = nav.borrow_mut().travel.take().expect("travelling");
			nav.borrow_mut().path.clear();
			let here = units.info(identity).and_then(|u| u.tile).unwrap_or(dest);
			movement_finish(vm, &t.entity, tile(&grid, vm, here)?)?;
			if t.attacked_on_arrival {
				resettle_after_free_attack(vm, &grid, &units, identity, &t.entity, t.last_empty)?;
			}
			Ok(false)
		}
	}
}
