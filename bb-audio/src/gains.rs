//! How loud a sound is in each ear, as the original's library and the driver under it
//! make it.
//!
//! The original plays through irrKlang 1.6 on DirectSound (its default "hardware
//! sound"), and under Proton DirectSound is Wine's (`dlls/dsound`). So:
//! - irrKlang turns a volume `v` into millibels as `trunc(1000 · ln v)`, at most 0,
//!   and -10000 (silence) at or below `e⁻¹⁰` (irrKlang `0x10014300`), and a pan `p`
//!   (1 is left) into `trunc(-10000 · p)`;
//! - Wine turns millibels into a gain as `2^(mB / 600)`; a pan attenuates the far
//!   ear by its value (`DSOUND_RecalcVolPan`);
//! - a sound at a world position (`Sound.play` with a position) is a 3D buffer:
//!   its distance from the listener, at least the minimum 512, grows only by the
//!   rolloff factor past it, and costs `2000 · log10(d / 512)` mB; its direction,
//!   against the listener's left (front × top), splits the gain between the ears as
//!   `√(1 − a)` and `√a` (`DSOUND_Calc3DBuffer`, stereo speakers at ∓90°).
//!
//! The game's listener (`0x543f20`) is at the camera's centre `(cx, cy, -1)`, facing
//! `(cx, cy, 1000)` (a direction, so it turns as the camera moves: the original's,
//! kept), with up `(0, 1, 0)`; a sound sits at `(x, y, 0)`. Before any scene sets it,
//! the listener is the library's default: at the origin, facing `(0, 0, 1)`.
//!
//! [`positional`] is the original's pan, kept for reference: this engine pans by the
//! sound's place on the screen instead ([`crate::pan`]), with the same rolloff.

/// Left and right gains, 1 is full.
pub type Gains = (f32, f32);

/// The minimum distance the game gives every positional sound.
const MIN_DISTANCE: f32 = 512.0;

/// The library's maximum distance (never changed by the game).
const MAX_DISTANCE: f32 = 1.0e9;

/// The silence floor, in millibels.
const SILENCE_MB: f64 = -10_000.0;

/// The rolloff factor the game sets from the screen's width at startup (`0x4adcc0`).
pub fn rolloff_for_width(width: u32) -> f32 {
	if width <= 1920 { 0.3 } else { 0.3 / (width as f32 / 1920.0) }
}

/// irrKlang's millibels for a volume.
pub fn volume_mb(v: f32) -> f64 {
	if !(f64::from(v) > (-10.0f64).exp()) {
		return SILENCE_MB;
	}
	(1000.0 * f64::from(v).ln()).trunc().clamp(SILENCE_MB, 0.0)
}

/// Wine's gain for `mb`.
fn gain(mb: f64) -> f32 {
	2.0f64.powf(mb / 600.0) as f32
}

/// A two-dimensional sound's gains: its volume and pan (1 left, -1 right).
pub fn flat(volume: f32, pan: f32) -> Gains {
	let mb = volume_mb(volume);
	if mb <= SILENCE_MB {
		return (0.0, 0.0);
	}
	let pan = f64::from((pan * -10_000.0).trunc());
	(gain(mb - pan.max(0.0)), gain(mb + pan.min(0.0)))
}

/// Where the listener is: the game's camera centre, or the library's default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Listener {
	Default,
	At(f32, f32),
}

type V3 = [f64; 3];

fn dot(a: V3, b: V3) -> f64 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn angle(a: V3, b: V3) -> f64 {
	let (la, lb) = (dot(a, a).sqrt(), dot(b, b).sqrt());
	if la == 0.0 || lb == 0.0 {
		return 0.0;
	}
	(dot(a, b) / (la * lb)).clamp(-1.0, 1.0).acos()
}

/// The listener's position and the direction it faces.
fn placement(listener: Listener) -> (V3, V3) {
	match listener {
		Listener::Default => ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
		Listener::At(x, y) => ([f64::from(x), f64::from(y), -1.0], [f64::from(x), f64::from(y), 1000.0]),
	}
}

/// From the listener to a sound at `at`.
fn towards(at: (f32, f32), position: V3) -> V3 {
	[f64::from(at.0) - position[0], f64::from(at.1) - position[1], 0.0 - position[2]]
}

/// A positional sound's whole gain, before it is split between the ears: its volume
/// and its distance from the listener, rolling off by `rolloff`.
pub(crate) fn distance_gain(volume: f32, at: (f32, f32), listener: Listener, rolloff: f32) -> f32 {
	let to = towards(at, placement(listener).0);
	let (min, max) = (f64::from(MIN_DISTANCE), f64::from(MAX_DISTANCE));
	let d = dot(to, to).sqrt().clamp(min, max);
	let d = min + (d - min) * f64::from(rolloff);
	let mb = volume_mb(volume) - ((d / min) * (d / min)).log10() * 1000.0;
	gain(mb)
}

/// The two ears' shares of a gain for a sound at `t` between the speakers (0 left, 1
/// right): `√(1 − t)` and `√t`.
pub(crate) fn split(gain: f32, t: f32) -> Gains {
	((1.0 - t).sqrt() * gain, t.sqrt() * gain)
}

/// A positional sound's gains as the original makes them: its volume, where it is, the
/// listener, the rolloff. Bypassed by default (see [`crate::pan`]); kept for reference.
pub fn positional(volume: f32, at: (f32, f32), listener: Listener, rolloff: f32) -> Gains {
	let (position, front) = placement(listener);
	let top = [0.0, 1.0, 0.0];
	let to = towards(at, position);
	let gain = distance_gain(volume, at, listener, rolloff);
	use std::f64::consts::{FRAC_PI_2, PI};
	let mut a = if to == [0.0; 3] {
		0.0
	} else {
		let left = cross(front, top);
		let mut a = angle(left, to);
		if angle(front, to) > FRAC_PI_2 {
			a = -a;
		}
		a -= FRAC_PI_2;
		if a < -PI {
			a += 2.0 * PI;
		}
		a
	};
	// Speakers at -90° (left) and 90° (right).
	if (-FRAC_PI_2..FRAC_PI_2).contains(&a) {
		split(gain, ((a + FRAC_PI_2) / PI) as f32)
	} else {
		if a < -FRAC_PI_2 {
			a += 2.0 * PI;
		}
		let t = ((a - FRAC_PI_2) / PI) as f32;
		(t.sqrt() * gain, (1.0 - t).sqrt() * gain)
	}
}

/// The volume (decibels) and panning (-1 left, 1 right) that make kira play a sound
/// at `gains` (kira pans as `√(1 − m)`, `√m`, times √2, with `m` its panning's share).
pub fn kira_settings(gains: Gains) -> (f32, f32) {
	let (l, r) = gains;
	let power = l * l + r * r;
	if power <= 0.0 {
		return (f32::NEG_INFINITY, 0.0);
	}
	let amplitude = (power / 2.0).sqrt();
	(20.0 * amplitude.log10(), 2.0 * (r * r / power) - 1.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn close(a: Gains, b: Gains) -> bool {
		(a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
	}

	#[test]
	fn volumes_follow_irrklang_and_wine() {
		assert_eq!(volume_mb(1.0), 0.0);
		assert_eq!(volume_mb(3.0), 0.0, "above 1 is 1");
		assert_eq!(volume_mb(0.0), -10_000.0);
		assert_eq!(volume_mb(0.5), -693.0);
		assert!(close(flat(1.0, 0.0), (1.0, 1.0)));
		assert!(close(flat(0.5, 0.0), (2.0f32.powf(-693.0 / 600.0), 2.0f32.powf(-693.0 / 600.0))));
		assert_eq!(flat(0.0, 0.0), (0.0, 0.0));
	}

	#[test]
	fn a_pan_attenuates_the_far_ear() {
		let (l, r) = flat(1.0, 0.0625);
		assert_eq!(l, 1.0, "+ pans left");
		assert!((r - 2.0f32.powf(-625.0 / 600.0)).abs() < 1e-6, "625 mB off the right: {r}");
		let (l, r) = flat(1.0, -1.0);
		assert!(l < 1e-4 && r == 1.0);
	}

	#[test]
	fn a_near_sound_is_full_and_a_far_one_quieter() {
		let listener = Listener::At(0.0, 0.0);
		let (l, r) = positional(1.0, (0.0, 100.0), listener, 0.3);
		assert!((l * l + r * r - 1.0).abs() < 1e-4, "within 512: the whole gain, split: {l}, {r}");
		let (l2, r2) = positional(1.0, (0.0, 3512.0), listener, 0.3);
		// 512 + 3000 · 0.3 = 1412: 2000 · log10(1412 / 512) mB off.
		let expected = 2.0f64.powf(-2000.0 * (1412.0f64 / 512.0).log10() / 600.0) as f32;
		assert!(((l2 * l2 + r2 * r2).sqrt() - expected).abs() < 1e-3, "{l2}, {r2} vs {expected}");
	}

	#[test]
	fn kira_reproduces_the_gains() {
		for gains in [(1.0, 1.0), (1.0, 0.25), (0.1, 0.7)] {
			let (db, panning) = kira_settings(gains);
			let amplitude = 10.0f32.powf(db / 20.0);
			let m = (panning + 1.0) * 0.5;
			let back = (amplitude * (1.0 - m).sqrt() * 2.0f32.sqrt(), amplitude * m.sqrt() * 2.0f32.sqrt());
			assert!(close(back, gains), "{gains:?} → {back:?}");
		}
	}

	#[test]
	fn wide_screens_roll_off_slower() {
		assert_eq!(rolloff_for_width(1920), 0.3);
		assert_eq!(rolloff_for_width(2560), 0.3 / (2560.0 / 1920.0));
	}
}
