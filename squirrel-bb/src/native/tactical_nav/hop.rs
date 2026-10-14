//! How a unit moves in a battle the player sees (`travel`, `teleport`; disassembled
//! `0x4c7ef0`, `0x4ca990`). It doesn't walk the path tile by tile: it hops
//! from where it stands, A, straight to the destination, B, in three phases on virtual
//! time:
//!
//! 1. lift-off: it rises 40 units over 0.14 s;
//! 2. flight: a straight line from 40 over A to 50 over B, 0.18 s per 126 units of it,
//!    with an arc of up to 3/4 of a quarter of the distance across (at most 45 units);
//! 3. landing: it drops the 50 units over 0.23 s.
//!
//! Each phase starts at the call that ended the one before (an overshoot isn't carried
//! over), and that call shows the finished phase's end. A travel's times are 5 times
//! shorter with faster movement; a teleport's are divided by its speed.
//!
//! How it paces is the host's choice ([`BattleMotion`]): over virtual time as the
//! original does, or one phase a call, which keeps the order of every script call and
//! paces a battle the same at any frame rate.

use squirrel::{Number, SQVM, SqStrongRef, SqTable};

/// How a unit's move in a battle is paced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BattleMotion {
	/// One phase a call (draw-exact battles and the combat goldens depend on it).
	#[default]
	Stepped,
	/// As the original's, over virtual time.
	Timed,
}

const LIFT_OFF: f32 = 0.14;
/// Flight seconds per [`FLIGHT_UNIT`] units.
const FLIGHT: f32 = 0.18;
const FLIGHT_UNIT: f32 = 126.0;
const LANDING: f32 = 0.23;
/// How high it lifts off, and starts its flight.
const LIFT: f32 = 40.0;
/// How high over the destination its flight ends, and its landing starts.
const LAND: f32 = 50.0;
/// Faster movement's time factor.
pub(super) const FASTER: f32 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
	LiftOff,
	Flight,
	Landing,
}

/// A hop from `from` to `to` (tile positions, level included).
pub(crate) struct Hop {
	pub phase: Phase,
	/// Virtual seconds when the phase started.
	since: f32,
	from: (f32, f32),
	to: (f32, f32),
	/// Multiplies every phase's time.
	time: f32,
}

impl Hop {
	pub fn new(from: (f64, f64), to: (f64, f64), time: f32, now: f32) -> Self {
		Hop {
			phase: Phase::LiftOff,
			since: now,
			from: (from.0 as f32, from.1 as f32),
			to: (to.0 as f32, to.1 as f32),
			time,
		}
	}

	fn flight(&self) -> ((f32, f32), (f32, f32)) {
		let p0 = (self.from.0, self.from.1 + LIFT);
		let p1 = (self.to.0, self.to.1 + LAND);
		(p0, (p1.0 - p0.0, p1.1 - p0.1))
	}

	/// How far into its phase it is at `now` (1 or more: done), or done at once when
	/// stepped.
	pub fn progress(&self, motion: BattleMotion, now: f32) -> f32 {
		if motion == BattleMotion::Stepped {
			return 1.0;
		}
		let length = match self.phase {
			Phase::LiftOff => LIFT_OFF,
			Phase::Flight => {
				let (_, d) = self.flight();
				(d.0 * d.0 + d.1 * d.1).sqrt() / FLIGHT_UNIT * FLIGHT
			}
			Phase::Landing => LANDING,
		};
		((now - self.since) / (length * self.time)).min(1.0)
	}

	/// Where the unit is `p` of the way through its phase.
	pub fn pose(&self, p: f32) -> (f64, f64) {
		let (x, y) = match self.phase {
			Phase::LiftOff => (self.from.0, self.from.1 + LIFT * p),
			Phase::Flight => {
				let (p0, d) = self.flight();
				let arc = (0.25 * d.0.abs()).min(60.0);
				(p0.0 + d.0 * p, p0.1 + d.1 * p + 3.0 * arc * (0.25 - (p - 0.5) * (p - 0.5)))
			}
			Phase::Landing => (self.to.0, self.to.1 + LAND * (1.0 - p)),
		};
		(f64::from(x), f64::from(y))
	}

	/// On to the next phase at `now`.
	pub fn next(&mut self, now: f32) {
		self.phase = match self.phase {
			Phase::LiftOff => Phase::Flight,
			Phase::Flight | Phase::Landing => Phase::Landing,
		};
		self.since = now;
	}
}

/// `Time.getVirtualTimeF()`: the hop's clock.
pub(super) fn virtual_now(vm: &SQVM) -> f32 {
	SqTable::root(vm)
		.get::<SqTable>("Time")
		.and_then(|t| t.get::<SqStrongRef>("getVirtualTimeF"))
		.and_then(|f| f.call_ret::<Number, _>(vm, ()).ok().flatten())
		.map_or(0.0, |n| n.0 as f32)
}

/// Whether `faction`'s moves are faster (`0x4c832b`): the player's (faction 1) with
/// `FasterPlayerMovement`, everyone else's with `FasterAIMovement`, set in either the
/// gameplay settings or their temporary ones.
pub(super) fn faster(vm: &SQVM, faction: i64) -> bool {
	let name = if faction == 1 { "FasterPlayerMovement" } else { "FasterAIMovement" };
	let set_in = |getter: &str| {
		SqTable::root(vm)
			.get::<SqTable>("Settings")
			.and_then(|s| s.get::<SqStrongRef>(getter))
			.and_then(|f| f.call_ret::<SqStrongRef, _>(vm, ()).ok().flatten())
			.and_then(|t| t.get_member::<bool>(vm, name))
			.unwrap_or(false)
	};
	set_in("getGameplaySettings") || set_in("getTempGameplaySettings")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_hop_lifts_flies_and_lands() {
		let mut hop = Hop::new((0.0, 0.0), (252.0, 0.0), 1.0, 10.0);
		assert!((hop.progress(BattleMotion::Timed, 10.07) - 0.5).abs() < 1e-3);
		assert_eq!(hop.pose(0.5), (0.0, 20.0));
		assert_eq!(hop.progress(BattleMotion::Timed, 10.2), 1.0);
		hop.next(10.2);
		// 40 up at the start, 50 up at the end, 0.75 × min(252 / 4, 60) = 45 more halfway.
		assert_eq!(hop.pose(0.0), (0.0, 40.0));
		assert_eq!(hop.pose(1.0), (252.0, 50.0));
		assert_eq!(hop.pose(0.5), (126.0, 90.0));
		// 252.2 units of flight: just over 0.36 s.
		assert!(hop.progress(BattleMotion::Timed, 10.2 + 0.36) < 1.0);
		assert_eq!(hop.progress(BattleMotion::Timed, 10.2 + 0.361), 1.0);
		hop.next(11.0);
		assert_eq!(hop.pose(0.0), (252.0, 50.0));
		assert_eq!(hop.pose(1.0), (252.0, 0.0));
		assert!(hop.progress(BattleMotion::Timed, 11.0 + 0.229) < 1.0);
		assert_eq!(hop.progress(BattleMotion::Timed, 11.0 + 0.231), 1.0);
	}

	#[test]
	fn stepped_hops_take_a_call_a_phase() {
		let hop = Hop::new((0.0, 0.0), (126.0, 0.0), 1.0, 0.0);
		assert_eq!(hop.progress(BattleMotion::Stepped, 0.0), 1.0);
	}

	#[test]
	fn faster_movement_is_five_times_quicker() {
		let hop = Hop::new((0.0, 0.0), (126.0, 0.0), FASTER, 0.0);
		assert_eq!(hop.progress(BattleMotion::Timed, 0.028), 1.0);
	}
}
