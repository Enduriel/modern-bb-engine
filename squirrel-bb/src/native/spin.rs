//! Sprites that turn by their `Torque` (disassembled 2026-10-14):
//! - Setting a brush sets the sprite's torque to the brush's own rotation speed
//!   (`0x52c03e`); setting the brush it already has, or an unknown one, leaves the
//!   torque alone. Scripts can set `Torque` afterwards (the swamp fog does).
//! - Each frame the engine turns every sprite with a torque by `(virtual time − the
//!   time it last turned) × torque` degrees (`0x52c136`), so a paused game stops them.
//!   A sprite without torque doesn't turn and keeps its last time. It turns as it's
//!   drawn, and only one that's visible and has a brush (`0x52c120`'s callers), so a
//!   hidden one keeps its last time too and catches up in one turn once shown (see
//!   `SpriteState::turns`).
//! - A rotation past ±360 starts again from 0: the original subtracts
//!   `(r ∓ 360) ± 360`, which in `f32` is `r` itself (give or take its rounding), not
//!   360.
//!
//! When a sprite first turns is not known (the original's last time starts at some
//! value set at creation), so here its first turn only records the time.

use crate::native::Sprites;

/// A sprite's turning: its `Torque`, in degrees per second of virtual time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spin {
	pub torque: f32,
	/// The virtual time it last turned at (the original's node field `+0x7c`).
	last: Option<f32>,
}

impl Spin {
	/// A sprite turning `torque` degrees per second.
	pub(crate) fn new(torque: f32) -> Spin {
		Spin { torque, last: None }
	}

	/// Brush `new` set on a sprite showing `old`: a different, known brush brings its
	/// rotation speed as the torque (`0x52bf50`).
	pub(crate) fn rebrush(&mut self, sprites: &Sprites, old: &str, new: &str) {
		if new != old {
			if let Some(speed) = sprites.with(|r| r.get(new).map(|(_, s)| s.rotation_speed)) {
				self.torque = speed;
			}
		}
	}

	/// `rotation` (degrees) turned to virtual time `now` (seconds).
	pub(crate) fn turn(&mut self, rotation: f32, now: f32) -> f32 {
		if self.torque == 0.0 {
			return rotation;
		}
		let last = self.last.replace(now).unwrap_or(now);
		let mut r = (now - last) * self.torque + rotation;
		if r < -360.0 {
			r -= (r + 360.0) - 360.0;
		} else if r > 360.0 {
			r -= (r - 360.0) + 360.0;
		}
		r
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn spin(torque: f32) -> Spin {
		Spin::new(torque)
	}

	#[test]
	fn turns_by_virtual_time_from_the_first_turn() {
		let mut s = spin(15.0);
		assert_eq!(s.turn(100.0, 10.0), 100.0, "the first turn only records the time");
		assert_eq!(s.turn(100.0, 12.0), 130.0);
		assert_eq!(s.turn(130.0, 12.0), 130.0, "a paused clock doesn't turn it");
		let mut back = spin(-15.0);
		back.turn(0.0, 0.0);
		assert_eq!(back.turn(0.0, 1.0), -15.0);
	}

	#[test]
	fn without_torque_it_keeps_its_last_time() {
		let mut s = spin(10.0);
		s.turn(0.0, 1.0);
		s.torque = 0.0;
		assert_eq!(s.turn(5.0, 3.0), 5.0);
		s.torque = 10.0;
		assert_eq!(s.turn(5.0, 4.0), 35.0, "3 s since it last turned");
	}

	#[test]
	fn past_a_full_turn_it_starts_again_from_zero() {
		let mut s = spin(100.0);
		s.turn(350.0, 0.0);
		assert_eq!(s.turn(350.0, 0.5), 0.0, "400 degrees is 0, not 40");
		let mut back = spin(-100.0);
		back.turn(-350.0, 0.0);
		assert_eq!(back.turn(-350.0, 0.5), 0.0);
		let mut edge = spin(10.0);
		edge.turn(355.0, 0.0);
		assert_eq!(edge.turn(355.0, 0.5), 360.0, "360 itself stays");
	}
}
