//! Footprints on the world map (`World.spawnFootprint`, `World.getAllFootprintsAtPos`)
//! and `World.getDirection8FromTo`, which scripts use to pick a footprint's brush.
//! Disassembled 2026-10-14.
//!
//! - `spawnFootprint(pos, brush, scale, stay, visibility, type)` (`0x4f0000`): a
//!   footprint with no stay time (a party's ordinary step) is only made within 3000
//!   units of the player's view; it then lasts `stay` seconds of virtual time.
//! - Expired ones go during `World.update()` (`0x4ef0ab`): from the oldest, each 60 s
//!   or more past its time is removed, stopping at the first that isn't. So a
//!   long-lasting footprint keeps every later one around until it goes. We apply the
//!   same rule whenever footprints are read.
//! - `getAllFootprintsAtPos(pos, count)` (`0x4f35b0`): `count` flags, one per
//!   footprint type, set when a footprint of that type within 100 units of `pos` is
//!   seen from the player's view: no farther from it than the vision radius times the
//!   footprint's visibility.
//! - Drawn (`0x4e49a0`) while `ShowTracking` is on and the footprint is seen that way,
//!   in the details layer with sort key 0, fading over the 60 s after it expires from
//!   0.8 opacity.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqArray, SqStrongRef, SqTable};

use super::{EntityMachine, vec_xy};
use crate::native::world_query;

type Machine = Rc<RefCell<EntityMachine>>;

/// Footprints with no stay time are only made this close to the player's view.
const SPAWN_REACH: f32 = 3000.0;
/// Seconds an expired footprint fades for before it goes.
const FADE: f32 = 60.0;
/// How close to the asked position a footprint counts (`getAllFootprintsAtPos`).
const NEAR: f32 = 100.0;
/// A fresh footprint's opacity.
const OPACITY: f32 = 0.8;

/// One footprint.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Footprint {
	pub pos: (f32, f32),
	/// Shared with the scenes a renderer reads.
	pub brush: std::rc::Rc<str>,
	pub scale: f32,
	/// Virtual time (s) its stay ends.
	pub expires: f32,
	/// Times the player's vision radius: how far away it can be seen.
	pub visibility: f32,
	/// `Const.World.FootprintsType`.
	pub kind: i64,
	/// Its entity id (the original's footprint is a scene entity, which takes one).
	pub id: i64,
}

/// The footprints, oldest first.
#[derive(Default)]
pub(crate) struct Footprints {
	list: Vec<Footprint>,
}

/// Distance in `f32`, as the original computes it.
fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
	let (dx, dy) = (a.0 - b.0, a.1 - b.1);
	f64::from(dx * dx + dy * dy).sqrt() as f32
}

impl Footprints {
	/// `spawnFootprint`, with the player's view at `view` and the virtual time `now`. A
	/// footprint that is made takes an id from `ids`.
	pub fn spawn(&mut self, footprint: Footprint, stay: f32, view: (f32, f32), now: f32, ids: &crate::native::EntityIds) {
		if stay == 0.0 && distance(view, footprint.pos) >= SPAWN_REACH {
			return;
		}
		self.list.push(Footprint {
			expires: now + stay,
			id: ids.next(),
			..footprint
		});
	}

	/// The footprints, oldest first.
	pub fn all(&self) -> &[Footprint] {
		&self.list
	}

	/// A footprint as a save had it (the newest last).
	pub fn push(&mut self, footprint: Footprint) {
		self.list.push(footprint);
	}

	/// Remove the expired ones the original's update would have by `now`.
	pub fn expire(&mut self, now: f32) {
		let gone = self.list.iter().take_while(|f| now - f.expires >= FADE).count();
		self.list.drain(..gone);
	}

	/// Whether the player's view at `view` with `vision` radius sees `f`.
	fn seen(f: &Footprint, view: (f32, f32), vision: f32) -> bool {
		vision * f.visibility >= distance(f.pos, view)
	}

	/// `getAllFootprintsAtPos`.
	pub fn types_at(&self, pos: (f32, f32), count: usize, view: (f32, f32), vision: f32) -> Vec<bool> {
		let mut found = vec![false; count];
		for f in &self.list {
			let Some(slot) = usize::try_from(f.kind).ok().filter(|&k| k < count) else {
				continue;
			};
			if !found[slot] && NEAR >= distance(pos, f.pos) && Self::seen(f, view, vision) {
				found[slot] = true;
			}
		}
		found
	}

	/// The footprints drawn at virtual time `now`: each with its opacity (0 to 255).
	pub fn drawn(&self, now: f32, view: (f32, f32), vision: f32) -> Vec<(Footprint, f32)> {
		self.list
			.iter()
			.filter(|f| Self::seen(f, view, vision))
			.map(|f| {
				let faded = (now - f.expires).max(0.0) / FADE;
				(f.clone(), (1.0 - faded) * OPACITY * 255.0)
			})
			.collect()
	}
}

/// `Time.getVirtualTimeF()`.
pub(super) fn virtual_time(vm: &SQVM) -> f32 {
	SqTable::root(vm)
		.get::<SqStrongRef>("Time")
		.and_then(|t| t.call_method_ret::<Number, _>(vm, "getVirtualTimeF", ()).ok().flatten())
		.map_or(0.0, |n| n.0 as f32)
}

/// The player's view: position and vision radius.
fn view_of(m: &Machine) -> ((f32, f32), f32) {
	let m = m.borrow();
	let v = &m.player_view;
	((v.pos.0 as f32, v.pos.1 as f32), v.vision_radius)
}

/// `World.spawnFootprint`, `World.getAllFootprintsAtPos` and
/// `World.getDirection8FromTo`.
pub(super) fn world_methods(m: &Machine, vm: &mut SQVM) -> Vec<(String, SqStrongRef)> {
	let spawn = {
		let m = m.clone();
		vm.make_closure(
			move |vm: &SQVM,
			      pos: Retained,
			      brush: &str,
			      scale: Number,
			      stay: Number,
			      visibility: Number,
			      kind: Number|
			      -> Result<(), String> {
				let (x, y) = vec_xy(vm, &pos.0, "spawnFootprint")?;
				let now = virtual_time(vm);
				let (view, _) = view_of(&m);
				let mut machine = m.borrow_mut();
				machine.footprints.expire(now);
				let ids = machine.ids.clone();
				crate::native::world_scene_changes::note();
				machine.footprints.spawn(
					Footprint {
						pos: (x as f32, y as f32),
						brush: brush.into(),
						scale: scale.0 as f32,
						expires: 0.0,
						visibility: visibility.0 as f32,
						kind: kind.0 as i64,
						id: 0,
					},
					stay.0 as f32,
					view,
					now,
					&ids,
				);
				Ok(())
			},
		)
	};
	let all_at_pos = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, pos: Retained, count: Number| -> Result<SqStrongRef, String> {
			let (x, y) = vec_xy(vm, &pos.0, "getAllFootprintsAtPos")?;
			let now = virtual_time(vm);
			let (view, vision) = view_of(&m);
			let found = {
				let mut machine = m.borrow_mut();
				machine.footprints.expire(now);
				machine
					.footprints
					.types_at((x as f32, y as f32), count.0.max(0.0) as usize, view, vision)
			};
			let mut out = SqArray::new(vm);
			for flag in found {
				out.push(flag);
			}
			Ok(out.to_strong())
		})
	};
	let direction8_from_to = vm.make_closure(|vm: &SQVM, from: Retained, to: Retained| -> Result<i64, String> {
		let native = "getDirection8FromTo";
		Ok(world_query::direction8(vec_xy(vm, &from.0, native)?, vec_xy(vm, &to.0, native)?))
	});
	vec![
		("spawnFootprint".to_string(), spawn),
		("getAllFootprintsAtPos".to_string(), all_at_pos),
		("getDirection8FromTo".to_string(), direction8_from_to),
	]
}

#[cfg(test)]
mod tests {
	use super::*;

	fn print(pos: (f32, f32), kind: i64) -> Footprint {
		Footprint {
			pos,
			brush: "world_footprints_human_01".into(),
			scale: 1.0,
			expires: 0.0,
			visibility: 1.0,
			kind,
			id: 0,
		}
	}

	#[test]
	fn ordinary_steps_are_only_made_near_the_player() {
		let mut f = Footprints::default();
		f.spawn(print((5000.0, 0.0), 1), 0.0, (0.0, 0.0), 10.0, &crate::native::EntityIds::default());
		assert!(f.list.is_empty(), "3000 units or more away");
		f.spawn(print((2999.0, 0.0), 1), 0.0, (0.0, 0.0), 10.0, &crate::native::EntityIds::default());
		f.spawn(print((5000.0, 0.0), 1), 30.0, (0.0, 0.0), 10.0, &crate::native::EntityIds::default());
		assert_eq!(f.list.len(), 2, "near, or lasting");
		assert_eq!(f.list[1].expires, 40.0);
	}

	#[test]
	fn expiry_stops_at_the_first_footprint_still_around() {
		let mut f = Footprints::default();
		f.spawn(print((0.0, 0.0), 1), 100.0, (0.0, 0.0), 0.0, &crate::native::EntityIds::default());
		f.spawn(print((0.0, 0.0), 1), 0.0, (0.0, 0.0), 0.0, &crate::native::EntityIds::default());
		f.expire(100.0);
		assert_eq!(f.list.len(), 2, "the first lasts until 160 s, and keeps the second");
		f.expire(160.0);
		assert!(f.list.is_empty());
	}

	#[test]
	fn the_lookout_sees_types_near_the_position_within_vision() {
		let mut f = Footprints::default();
		f.spawn(print((50.0, 0.0), 2), 30.0, (0.0, 0.0), 0.0, &crate::native::EntityIds::default());
		f.spawn(print((150.0, 0.0), 3), 30.0, (0.0, 0.0), 0.0, &crate::native::EntityIds::default());
		f.spawn(print((0.0, 900.0), 4), 30.0, (0.0, 0.0), 0.0, &crate::native::EntityIds::default());
		let at = f.types_at((0.0, 0.0), 6, (0.0, 0.0), 500.0);
		assert_eq!(at, [false, false, true, false, false, false]);
		// Seen only within vision × visibility of the player's view.
		assert_eq!(f.types_at((0.0, 900.0), 6, (0.0, 0.0), 500.0)[4], false);
		assert_eq!(f.types_at((0.0, 900.0), 6, (0.0, 0.0), 1000.0)[4], true);
		assert_eq!(f.types_at((0.0, 0.0), 2, (0.0, 0.0), 500.0), [false, false]);
	}

	#[test]
	fn footprints_fade_after_their_stay() {
		let mut f = Footprints::default();
		f.spawn(print((0.0, 0.0), 1), 10.0, (0.0, 0.0), 0.0, &crate::native::EntityIds::default());
		assert_eq!(f.drawn(5.0, (0.0, 0.0), 100.0)[0].1, 0.8 * 255.0);
		assert_eq!(f.drawn(40.0, (0.0, 0.0), 100.0)[0].1, 0.5 * 0.8 * 255.0);
	}
}
