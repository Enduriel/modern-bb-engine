//! `entity.move(destination, speed)`: one frame of a world entity's movement, as in
//! the original's disassembly (2026-10-13).
//!
//! The entity steps `speed` px straight towards `destination`, or lands on it when the
//! step would reach it. It returns `true` while still on the way, and `false` once it
//! has arrived, when it is already there, or when blocked. Unless the entity ignores
//! collision, it never moves onto impassable or ocean tiles (terrain type ≤ 1); and an
//! entity standing on such a tile with speed 0 is given a speed of 30, so it can get
//! off. Arithmetic is `f32`.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqStrongRef, SqTable};

use super::{EntityMachine, tile_under, vec_xy};

/// The highest terrain type an entity can't enter (`Impassable` = 0, `Ocean` = 1).
const BLOCKING_TERRAIN: i64 = 1;

type Machine = Rc<RefCell<EntityMachine>>;

/// The terrain type of axial tile `(q, r)` (`World.getTile(q, r).Type`).
fn terrain_at(vm: &SQVM, q: i64, r: i64) -> Result<i64, String> {
	let world = SqTable::root(vm).get::<SqStrongRef>("World").ok_or("World table missing")?;
	let tile = world
		.call_method_ret::<SqStrongRef, _>(vm, "getTile", (q, r))?
		.ok_or("move: no tile there")?;
	tile.get_member::<i64>(vm, "Type").ok_or_else(|| "move: tile has no Type".to_string())
}

/// The terrain type under a world pixel position.
fn terrain_under(vm: &SQVM, x: f32, y: f32) -> Result<i64, String> {
	let create_vec = SqTable::root(vm).get::<SqStrongRef>("createVec").ok_or("createVec missing")?;
	let pos = create_vec
		.call_ret::<SqStrongRef, _>(vm, (f64::from(x), f64::from(y)))?
		.ok_or("createVec returned null")?;
	let (q, r) = tile_under(vm, &pos)?;
	terrain_at(vm, q, r)
}

/// Where one call of `move` puts the entity, or `None` if it doesn't move.
/// `blocked(x, y)` tells whether the tile under a position can't be entered.
fn step(
	pos: (f32, f32),
	dest: (f32, f32),
	speed: f32,
	on_blocking_tile: bool,
	mut blocked: impl FnMut(f32, f32) -> Result<bool, String>,
) -> Result<(Option<(f32, f32)>, bool), String> {
	if pos == dest {
		return Ok((None, false));
	}
	let speed = if on_blocking_tile && speed == 0.0 { 30.0 } else { speed };
	let (dx, dy) = (dest.0 - pos.0, dest.1 - pos.1);
	let dist = f64::from(dx * dx + dy * dy).sqrt() as f32;
	if speed >= dist {
		let to = if blocked(dest.0, dest.1)? { None } else { Some(dest) };
		return Ok((to, false));
	}
	let inv = 1.0 / dist;
	let next = (inv * dx * speed + pos.0, inv * dy * speed + pos.1);
	if blocked(next.0, next.1)? {
		return Ok((None, false));
	}
	Ok((Some(next), true))
}

/// The injected `move` native.
pub(super) fn entity_methods(m: &Machine, vm: &mut SQVM) -> Vec<(&'static str, SqStrongRef)> {
	let move_to = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, dest: Retained, speed: Number| -> Result<bool, String> {
			let (pos, tile, ignore_collision) = {
				let machine = m.borrow();
				let rec = machine.record(vm.this_identity()).ok_or("move: entity not registered")?;
				(rec.pos, (rec.q, rec.r), rec.ignore_collision)
			};
			let pos = (pos.0 as f32, pos.1 as f32);
			let (dx, dy) = vec_xy(vm, &dest.0);
			let on_blocking_tile = !ignore_collision && terrain_at(vm, tile.0, tile.1)? <= BLOCKING_TERRAIN;
			let blocked = |x: f32, y: f32| -> Result<bool, String> {
				Ok(!ignore_collision && terrain_under(vm, x, y)? <= BLOCKING_TERRAIN)
			};
			let (to, moving) = step(pos, (dx as f32, dy as f32), speed.0 as f32, on_blocking_tile, blocked)?;
			if let Some((x, y)) = to {
				let entity = m.borrow().record(vm.this_identity()).map(|rec| rec.entity.clone());
				let create_vec = SqTable::root(vm).get::<SqStrongRef>("createVec").ok_or("createVec missing")?;
				let new_pos = create_vec
					.call_ret::<SqStrongRef, _>(vm, (f64::from(x), f64::from(y)))?
					.ok_or("createVec returned null")?;
				if let Some(e) = entity {
					e.call_method(vm, "setPos", (new_pos,))?;
				}
			}
			Ok(moving)
		})
	};
	vec![("move", move_to)]
}

#[cfg(test)]
mod tests {
	use super::step;

	fn open(_: f32, _: f32) -> Result<bool, String> {
		Ok(false)
	}

	#[test]
	fn steps_towards_then_lands() {
		// 3-4-5 triangle: a step of 2.5 along it.
		let (to, moving) = step((0.0, 0.0), (3.0, 4.0), 2.5, false, open).unwrap();
		assert_eq!(to, Some((1.5, 2.0)));
		assert!(moving);
		// A step that reaches the destination lands on it and reports arrival.
		let (to, moving) = step((0.0, 0.0), (3.0, 4.0), 5.0, false, open).unwrap();
		assert_eq!(to, Some((3.0, 4.0)));
		assert!(!moving);
		// Already there.
		assert_eq!(step((1.0, 1.0), (1.0, 1.0), 9.0, false, open).unwrap(), (None, false));
	}

	#[test]
	fn blocked_terrain_stops_it() {
		let wall = |_: f32, _: f32| Ok(true);
		assert_eq!(step((0.0, 0.0), (100.0, 0.0), 10.0, false, wall).unwrap(), (None, false));
		// Stuck on a blocking tile with no speed: it gets 30 px.
		let (to, _) = step((0.0, 0.0), (100.0, 0.0), 0.0, true, open).unwrap();
		assert_eq!(to, Some((30.0, 0.0)));
	}
}
