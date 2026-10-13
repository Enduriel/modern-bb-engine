//! `navigator.teleport(unit, tile, onDone, tag, flag[, speed])`: moving a unit straight
//! to a tile, not along a path (knockbacks, pulls, jumps, an alp's or a hexe's
//! teleport), as the original does it (disassembled).
//!
//! It refuses (answers false) while a unit travels or switches places, onto the unit's
//! own tile or an occupied one, and for a unit already teleporting. Otherwise
//! `onMovementStart(tile, 0)` runs at once. At speed 0, or when the player sees
//! neither end, the unit lands at once: it takes the tile, then `onMovementFinish(tile)`
//! and `onDone(unit, tag)` run. Otherwise it animates over the next frames, driven by
//! `Tactical.update()`: it lifts off and takes the tile, flies, and lands, which can
//! discover it for the player, then the same two calls. Headless each phase takes one
//! update; the original times them (0.14 s to lift off at speed 1, then by distance),
//! so only frame timing differs. The unit counts as travelling (`isTravelling`) until
//! both calls are done.
//!
//! `switchEntities(a, b, onDone, tag, speed)` swaps two units the same way: both start
//! moving, and if the player sees neither tile they swap at once and `onDone(tag)` runs;
//! otherwise both leave their tiles and fly as two teleports.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqStrongRef};

use super::NavState;
use super::travel::{movement_finish, movement_start, seen_by_player, tile};
use crate::native::tactical_entities::unit_identity;
use crate::native::tactical_grid;

/// A teleport in progress.
pub(crate) struct Teleport {
	pub identity: i64,
	entity: SqStrongRef,
	dest: usize,
	/// `onDone`, if not null.
	callback: Option<SqStrongRef>,
	tag: SqStrongRef,
	phase: Phase,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
	LiftOff,
	Flying,
	Landing,
	/// Landed; its calls are running (it still counts as travelling).
	Done,
}

fn is_null(vm: &SQVM, value: &SqStrongRef) -> bool {
	matches!(value.get_value::<Option<i64>>(vm), Some(None))
}

/// `teleport(...)` with its 5 or 6 arguments.
pub(super) fn teleport(nav: &Rc<RefCell<NavState>>, vm: &SQVM, args: &[Retained]) -> Result<bool, String> {
	if args.len() != 5 && args.len() != 6 {
		return Err("teleport: wrong number of parameters".to_string());
	}
	let speed = match args.get(5) {
		Some(s) => s.0.get_value::<Number>(vm).ok_or("teleport: expected a speed")?.0 as f32,
		None => 1.0,
	};
	let (grid, units) = {
		let st = nav.borrow();
		if st.travel.is_some() {
			return Ok(false);
		}
		(st.grid.clone(), st.units.clone())
	};
	let identity = unit_identity(vm, &args[0].0);
	let dest = tactical_grid::tile_id_of(&grid, vm, &args[1].0).ok_or("teleport: expected a tile")?;
	let (Some(mover), Some(start)) = (units.entity(identity), units.info(identity).and_then(|u| u.tile)) else {
		return Ok(false);
	};
	if start == dest || grid.borrow().tiles[dest].occupant.is_some() {
		return Ok(false);
	}
	if nav.borrow().teleports.iter().any(|t| t.identity == identity) {
		return Ok(false);
	}
	let callback = (!is_null(vm, &args[2].0)).then(|| args[2].0.clone());
	let tag = args[3].0.clone();

	movement_start(vm, &mover, tile(&grid, vm, start)?, 0)?;
	if speed != 0.0 && (seen_by_player(&grid, start) || seen_by_player(&grid, dest)) {
		nav.borrow_mut().teleports.push(Teleport {
			identity,
			entity: mover,
			dest,
			callback,
			tag,
			phase: Phase::LiftOff,
		});
		return Ok(true);
	}
	units.move_to(identity, dest)?;
	land(vm, &grid, &mover, dest, callback.as_ref(), &tag)?;
	Ok(true)
}

/// `switchEntities(a, b, onDone, tag, speed)`.
pub(super) fn switch_entities(nav: &Rc<RefCell<NavState>>, vm: &SQVM, args: &[Retained]) -> Result<bool, String> {
	if args.len() != 5 {
		return Err("switchEntities: wrong number of parameters".to_string());
	}
	let (grid, units) = {
		let st = nav.borrow();
		if st.travel.is_some() {
			return Ok(false);
		}
		(st.grid.clone(), st.units.clone())
	};
	let (a, b) = (unit_identity(vm, &args[0].0), unit_identity(vm, &args[1].0));
	if a == b || nav.borrow().teleports.iter().any(|t| t.identity == a || t.identity == b) {
		return Ok(false);
	}
	let placed = |id: i64| Some((units.entity(id)?, units.info(id)?.tile?));
	let (Some((ea, ta)), Some((eb, tb))) = (placed(a), placed(b)) else {
		return Ok(false);
	};
	let callback = (!is_null(vm, &args[2].0)).then(|| args[2].0.clone());
	let tag = args[3].0.clone();

	movement_start(vm, &ea, tile(&grid, vm, ta)?, 0)?;
	movement_start(vm, &eb, tile(&grid, vm, tb)?, 0)?;
	if !seen_by_player(&grid, ta) && !seen_by_player(&grid, tb) {
		units.lift(b)?;
		units.move_to(a, tb)?;
		units.move_to(b, ta)?;
		movement_finish(vm, &ea, tile(&grid, vm, tb)?)?;
		movement_finish(vm, &eb, tile(&grid, vm, ta)?)?;
		if let Some(f) = &callback {
			f.call(vm, (tag.clone(),)).map_err(|e| format!("switchEntities callback: {e}"))?;
		}
		return Ok(true);
	}
	units.lift(a)?;
	units.lift(b)?;
	let mut st = nav.borrow_mut();
	for (identity, entity, dest) in [(a, ea, tb), (b, eb, ta)] {
		st.teleports.push(Teleport {
			identity,
			entity,
			dest,
			callback: callback.clone(),
			tag: tag.clone(),
			phase: Phase::LiftOff,
		});
	}
	Ok(true)
}

/// The landing calls: `onMovementFinish(tile)`, then `onDone(unit, tag)`.
fn land(
	vm: &SQVM,
	grid: &tactical_grid::Grid,
	mover: &SqStrongRef,
	here: usize,
	callback: Option<&SqStrongRef>,
	tag: &SqStrongRef,
) -> Result<(), String> {
	movement_finish(vm, mover, tile(grid, vm, here)?)?;
	if let Some(f) = callback {
		f.call(vm, (mover.clone(), tag.clone())).map_err(|e| format!("teleport callback: {e}"))?;
	}
	Ok(())
}

/// `Tactical.update()`: each teleport's next phase, in the order they started. One
/// started during this update waits for the next.
pub(super) fn update(nav: &Rc<RefCell<NavState>>, vm: &SQVM) -> Result<(), String> {
	let ids: Vec<i64> = nav.borrow().teleports.iter().map(|t| t.identity).collect();
	for id in ids {
		advance(nav, vm, id)?;
	}
	Ok(())
}

fn advance(nav: &Rc<RefCell<NavState>>, vm: &SQVM, identity: i64) -> Result<(), String> {
	let (grid, units) = {
		let st = nav.borrow();
		(st.grid.clone(), st.units.clone())
	};
	let phase_of = |nav: &Rc<RefCell<NavState>>| nav.borrow().teleports.iter().find(|t| t.identity == identity).map(|t| (t.phase, t.dest));
	let Some((phase, dest)) = phase_of(nav) else { return Ok(()) };
	let set_phase = |p: Phase| {
		if let Some(t) = nav.borrow_mut().teleports.iter_mut().find(|t| t.identity == identity) {
			t.phase = p;
		}
	};
	match phase {
		Phase::LiftOff => {
			units.move_to(identity, dest)?;
			set_phase(Phase::Flying);
		}
		Phase::Flying => {
			if seen_by_player(&grid, dest) {
				units.discover(vm, identity)?;
			}
			set_phase(Phase::Landing);
		}
		Phase::Landing => {
			set_phase(Phase::Done);
			let (entity, callback, tag) = {
				let st = nav.borrow();
				let t = st.teleports.iter().find(|t| t.identity == identity).expect("teleporting");
				(t.entity.clone(), t.callback.clone(), t.tag.clone())
			};
			let here = units.info(identity).and_then(|u| u.tile).unwrap_or(dest);
			let landed = land(vm, &grid, &entity, here, callback.as_ref(), &tag);
			nav.borrow_mut().teleports.retain(|t| t.identity != identity);
			landed?;
		}
		Phase::Done => {}
	}
	Ok(())
}
