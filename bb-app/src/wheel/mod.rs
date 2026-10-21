//! The mouse wheel in whole notches, as the original takes it. Its window code (SFML's)
//! makes a notch of every 120 of the system's wheel delta and drops what's short of one
//! (`MouseWheelMoved`, `delta / 120`; the game reads no other wheel event, `0x5b57d8`),
//! and each notch goes to the UI or to the states. Here a wheel's lines count as
//! notches, and a touchpad's pixels at ten logical pixels a notch (a wheel click is ten
//! in Wayland's pointer protocol, and a line is ten points on macOS); what's short of a
//! notch waits for the rest.
//!
//! macOS also takes pixels from a wheel that turns in steps ([`mac`]): Parsec's host
//! posts its client's wheel delta as pixels, unscaled, and a client's notch is Windows'
//! wheel delta, 120 (Parsec's convention; `BB_INPUT_LOG` shows what arrives). At ten a
//! notch that made twelve of each; a stepped wheel's pixels count 120 a notch instead.

#[cfg(target_os = "macos")]
mod mac;

use winit::event::MouseScrollDelta;

/// Logical pixels of a touchpad's scroll that make one notch.
pub const PIXELS_PER_NOTCH: f64 = 10.0;

/// Logical pixels of a stepped wheel's turn that make one notch where the system hands
/// it on as pixels (Windows' `WHEEL_DELTA`).
pub const WHEEL_DELTA: f64 = 120.0;

/// Logical pixels a notch for the pixels of the wheel event being handled now:
/// [`WHEEL_DELTA`] for a stepped wheel's (macOS, see [`mac`]), else [`PIXELS_PER_NOTCH`].
pub fn pixels_per_notch() -> f64 {
	#[cfg(target_os = "macos")]
	if mac::stepped() {
		return WHEEL_DELTA;
	}
	PIXELS_PER_NOTCH
}

/// What the system says of the wheel event being handled now (macOS), for a log.
pub fn describe_event() -> String {
	#[cfg(target_os = "macos")]
	return mac::describe();
	#[cfg(not(target_os = "macos"))]
	String::new()
}

/// A wheel turn in notches: lines as they are, a touchpad's physical pixels in logical
/// ones (at `scale_factor`) over `per_notch` (see [`pixels_per_notch`]). Positive is up,
/// away from the player.
pub fn notches(delta: &MouseScrollDelta, scale_factor: f64, per_notch: f64) -> f64 {
	match delta {
		MouseScrollDelta::LineDelta(_, y) => f64::from(*y),
		MouseScrollDelta::PixelDelta(p) => p.y / scale_factor.max(f64::MIN_POSITIVE) / per_notch.max(f64::MIN_POSITIVE),
	}
}

/// The wheel's turn so far, short of a whole notch.
#[derive(Debug, Default)]
pub struct Notches {
	rest: f64,
}

impl Notches {
	/// `turn` more notches (positive up): the whole notches they complete, signed. A turn
	/// the other way forgets what was left of the last one.
	pub fn add(&mut self, turn: f64) -> i32 {
		if turn * self.rest < 0.0 {
			self.rest = 0.0;
		}
		self.rest += turn;
		let whole = self.rest.trunc();
		self.rest -= whole;
		whole as i32
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use winit::dpi::PhysicalPosition;

	#[test]
	fn a_wheel_click_is_a_notch() {
		let mut n = Notches::default();
		assert_eq!(n.add(notches(&MouseScrollDelta::LineDelta(0.0, 1.0), 1.0, PIXELS_PER_NOTCH)), 1);
		assert_eq!(n.add(notches(&MouseScrollDelta::LineDelta(0.0, -1.0), 2.0, PIXELS_PER_NOTCH)), -1);
		assert_eq!(n.add(notches(&MouseScrollDelta::LineDelta(0.0, 3.0), 1.0, PIXELS_PER_NOTCH)), 3, "several at once");
	}

	#[test]
	fn parts_of_a_notch_add_up() {
		// A high-resolution wheel's eighths.
		let mut n = Notches::default();
		let turns: Vec<i32> = (0..16).map(|_| n.add(0.125)).collect();
		assert_eq!(turns.iter().sum::<i32>(), 2);
		assert_eq!(turns.iter().filter(|&&t| t == 1).count(), 2);
		// A touchpad's pixels at a scale of 2: 20 physical pixels a notch.
		let mut n = Notches::default();
		let px = |y: f64| notches(&MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, y)), 2.0, PIXELS_PER_NOTCH);
		assert_eq!(n.add(px(15.0)), 0);
		assert_eq!(n.add(px(15.0)), 1);
		assert_eq!(n.add(px(-5.0)), 0, "turning back forgets the rest");
		assert_eq!(n.add(px(-15.0)), -1);
	}

	#[test]
	fn a_stepped_wheels_pixels_are_windows_deltas() {
		// Parsec's host on a Retina Mac: 120 points a notch, 240 physical pixels.
		let mut n = Notches::default();
		let px = |y: f64| notches(&MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, y)), 2.0, WHEEL_DELTA);
		assert_eq!(n.add(px(240.0)), 1, "a notch, not twelve");
		assert_eq!(n.add(px(-480.0)), -2);
		// A high-resolution wheel's quarter notches add up.
		let turns: i32 = (0..8).map(|_| n.add(px(60.0))).sum();
		assert_eq!(turns, 2);
	}
}
