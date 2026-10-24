//! `navigator.teleport(unit, tile, onDone, tag, flag[, speed])`: moving a unit straight
//! to a tile, not along a path (knockbacks, pulls, jumps, an alp's or a hexe's
//! teleport), as the original does it (disassembled).
//!
//! It refuses (answers false) while a unit travels or switches places, onto the unit's
//! own tile or an occupied one, and for a unit already teleporting. Otherwise it ends
//! the unit's shake and `onMovementStart(tile, 0)` runs at once. At speed 0, or when the
//! player sees neither end, the unit lands at once: it takes the tile, then
//! `onMovementFinish(tile)` and `onDone(unit, tag)` run. Otherwise it hops there (see
//! `hop`, its times divided by the speed), driven by `Tactical.update()`: it lifts off
//! and takes the tile (fading out or in, with `flag`, if it leaves or enters the
//! player's view), flies, and lands, which can discover it for the player, then the same
//! two calls. The unit counts as travelling (`isTravelling`) until both calls are done.
//!
//! `switchEntities(a, b, onDone, tag, speed)` swaps two units the same way: both start
//! moving, and if the player sees neither tile they swap at once and `onDone(tag)` runs
//! once; otherwise both leave their tiles and fly as two teleports, each landing calling
//! `onDone(unit, tag)`. Off their tiles, they are still drawn with the units while they
//! rise (see `Units::lift_off`), until they take the other's tile in the air. A callback written for the landings therefore fails out of sight,
//! silently, as in the original (probed; see `on_done`); mods depend on it.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqStrongRef};

use super::NavState;
use super::hop::{self, Hop, Phase};
use super::travel::{movement_finish, movement_start, seen_by_player, tile};
use crate::native::tactical_entities::unit_identity;
use crate::native::tactical_grid;

/// A teleport in progress.
pub(crate) struct Teleport {
	pub identity: i64,
	entity: SqStrongRef,
	start: usize,
	dest: usize,
	/// `onDone`, if not null, and what it's handed.
	callback: Option<(SqStrongRef, SqStrongRef)>,
	hop: Hop,
	speed: f32,
	/// The script's `flag`: fade at the edge of the player's view.
	fades: bool,
	/// Landed; its calls are running (it still counts as travelling).
	done: bool,
}

impl Teleport {
	#[allow(clippy::too_many_arguments)]
	fn new(grid: &tactical_grid::Grid, identity: i64, entity: SqStrongRef, start: usize, dest: usize, callback: Option<(SqStrongRef, SqStrongRef)>, speed: f32, fades: bool, now: f32) -> Self {
		let at = |id: usize| grid.borrow().tiles[id].world_pos();
		Teleport {
			identity,
			entity,
			start,
			dest,
			callback,
			hop: Hop::new(at(start), at(dest), 1.0 / speed, now),
			speed,
			fades,
			done: false,
		}
	}
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
	if nav.borrow().travel.is_some() {
		return Ok(false);
	}
	let grid = nav.borrow().grid.clone();
	let identity = unit_identity(vm, &args[0].0);
	let dest = tactical_grid::tile_id_of(&grid, vm, &args[1].0).ok_or("teleport: expected a tile")?;
	let callback = (!is_null(vm, &args[2].0)).then(|| (args[2].0.clone(), args[3].0.clone()));
	let fades = args[4].0.get_value::<bool>(vm).unwrap_or(false);
	start(nav, vm, identity, dest, callback, speed, fades)
}

/// Teleport unit `identity` to tile `dest` (`0x4ca7a0`), as `teleport` does once its
/// arguments are read; also the re-settle after free attacks on arrival. Whether it
/// went.
pub(super) fn start(
	nav: &Rc<RefCell<NavState>>,
	vm: &SQVM,
	identity: i64,
	dest: usize,
	callback: Option<(SqStrongRef, SqStrongRef)>,
	speed: f32,
	fades: bool,
) -> Result<bool, String> {
	let (grid, units) = {
		let st = nav.borrow();
		(st.grid.clone(), st.units.clone())
	};
	let (Some(mover), Some(start)) = (units.entity(identity), units.info(identity).and_then(|u| u.tile)) else {
		return Ok(false);
	};
	if start == dest || grid.borrow().tiles[dest].occupant.is_some() {
		return Ok(false);
	}
	if nav.borrow().teleports.iter().any(|t| t.identity == identity) {
		return Ok(false);
	}

	units.cancel_shake(vm, identity);
	movement_start(vm, &mover, tile(&grid, vm, start)?, 0)?;
	if speed != 0.0 && (seen_by_player(&grid, start) || seen_by_player(&grid, dest)) {
		let t = Teleport::new(&grid, identity, mover, start, dest, callback, speed, fades, hop::virtual_now(vm));
		nav.borrow_mut().teleports.push(t);
		return Ok(true);
	}
	units.move_to(identity, dest)?;
	land(nav, vm, &grid, &mover, dest, callback.as_ref())?;
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
	// Both units' shakes end first (`0x4ca1e8`), as a teleport's does.
	units.cancel_shake(vm, a);
	units.cancel_shake(vm, b);
	let speed = args[4].0.get_value::<Number>(vm).map_or(1.0, |n| n.0 as f32);
	let landed_with = callback.clone().map(|f| (f, tag.clone()));

	movement_start(vm, &ea, tile(&grid, vm, ta)?, 0)?;
	movement_start(vm, &eb, tile(&grid, vm, tb)?, 0)?;
	if !seen_by_player(&grid, ta) && !seen_by_player(&grid, tb) {
		units.lift(b)?;
		units.move_to(a, tb)?;
		units.move_to(b, ta)?;
		movement_finish(vm, &ea, tile(&grid, vm, tb)?)?;
		movement_finish(vm, &eb, tile(&grid, vm, ta)?)?;
		// `onDone(tag)`, where the landings hand `onDone(unit, tag)`; see `on_done`.
		if let Some(f) = &callback {
			on_done(nav, |this| f.call_with_this(vm, this, (tag.clone(),)));
		}
		return Ok(true);
	}
	units.lift_off(a)?;
	units.lift_off(b)?;
	let now = hop::virtual_now(vm);
	let speed = if speed > 0.0 { speed } else { 1.0 };
	let mut st = nav.borrow_mut();
	for (identity, entity, start, dest) in [(a, ea, ta, tb), (b, eb, tb, ta)] {
		st.teleports.push(Teleport::new(&grid, identity, entity, start, dest, landed_with.clone(), speed, false, now));
	}
	Ok(true)
}

/// Run `onDone` as the original's Sqrat callbacks do (`0x4cc010`, `0x4cc160`): `this` is
/// the navigator, and an error in it is reported and goes no further (probed). A
/// callback that takes other arguments than it is handed can't even be called, which
/// Squirrel doesn't report with script on the stack: so `switchEntities` out of the
/// player's sight, which hands `onDone(tag)` only, silently never runs a callback of
/// `(unit, tag)`, and still swaps and answers true. Mods depend on this quirk: Reforged
/// passes null there and calls its callback from `onMovementFinish` itself.
fn on_done(nav: &Rc<RefCell<NavState>>, call: impl FnOnce(&SqStrongRef) -> Result<(), String>) {
	// The navigator calls these, so it is installed.
	let this = nav.borrow().instance.clone();
	if let Some(this) = this {
		drop(call(&this));
	}
}

/// The landing calls: `onMovementFinish(tile)`, then `onDone(unit, tag)` (see `on_done`).
fn land(
	nav: &Rc<RefCell<NavState>>,
	vm: &SQVM,
	grid: &tactical_grid::Grid,
	mover: &SqStrongRef,
	here: usize,
	callback: Option<&(SqStrongRef, SqStrongRef)>,
) -> Result<(), String> {
	movement_finish(vm, mover, tile(grid, vm, here)?)?;
	if let Some((f, tag)) = callback {
		on_done(nav, |this| f.call_with_this(vm, this, (mover.clone(), tag.clone())));
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
	let (grid, units, motion) = {
		let st = nav.borrow();
		(st.grid.clone(), st.units.clone(), st.motion)
	};
	let now = hop::virtual_now(vm);
	let state = |nav: &Rc<RefCell<NavState>>| {
		nav.borrow().teleports.iter().find(|t| t.identity == identity).map(|t| (t.hop.phase, t.hop.progress(motion, now), t.done, t.start, t.dest, t.speed, t.fades))
	};
	let Some((phase, p, done, start, dest, speed, fades)) = state(nav) else { return Ok(()) };
	if done {
		return Ok(());
	}
	let with = |f: &mut dyn FnMut(&mut Teleport)| {
		if let Some(t) = nav.borrow_mut().teleports.iter_mut().find(|t| t.identity == identity) {
			f(t);
		}
	};
	let pose = |p: f32| {
		let mut at = (0.0, 0.0);
		with(&mut |t| at = t.hop.pose(p));
		at
	};
	if p < 1.0 {
		units.set_pos(identity, pose(p));
		return Ok(());
	}
	match phase {
		Phase::LiftOff => {
			units.move_to(identity, dest)?;
			units.set_flying(identity, true);
			units.set_pos(identity, pose(1.0));
			if fades {
				let ms = (speed * 500.0) as i64;
				match (seen_by_player(&grid, start), seen_by_player(&grid, dest)) {
					(true, false) => units.fade(vm, identity, ms, true),
					(false, true) => units.fade(vm, identity, ms, false),
					_ => {}
				}
			}
			with(&mut |t| t.hop.next(now));
		}
		Phase::Flight => {
			units.set_flying(identity, false);
			with(&mut |t| t.hop.next(now));
			units.set_pos(identity, pose(0.0));
			if seen_by_player(&grid, dest) {
				units.discover(vm, identity)?;
			}
		}
		Phase::Landing => {
			with(&mut |t| t.done = true);
			if fades && !seen_by_player(&grid, dest) {
				units.opaque(vm, identity);
			}
			let (entity, callback) = {
				let st = nav.borrow();
				let t = st.teleports.iter().find(|t| t.identity == identity).expect("teleporting");
				(t.entity.clone(), t.callback.clone())
			};
			let here = units.info(identity).and_then(|u| u.tile).unwrap_or(dest);
			units.set_pos(identity, grid.borrow().tiles[here].world_pos());
			let landed = land(nav, vm, &grid, &entity, here, callback.as_ref());
			nav.borrow_mut().teleports.retain(|t| t.identity != identity);
			landed?;
		}
	}
	Ok(())
}
