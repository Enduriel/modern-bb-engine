//! A projectile in flight (`spawnProjectileEffect`; the original's `0x4bcd70`, its
//! update `0x4bd050`; disassembled 2026-10-14):
//! - it flies for `min(0.75, 0.0007 · d) · timeScale` seconds of virtual time, `d` the
//!   distance to the target tile with the target 5 units lower;
//! - from tile to tile in a straight line, lifted by an arc: `2h · (0.25 − (t − 0.5)²)`
//!   at `t` of the way, `h = min(260, 0.19 · |dx|)`;
//! - rotated, it points at the target from where it is; otherwise it turns by its
//!   brush's rotation speed (reversed when flipped, which also mirrors it);
//! - within `200 / timeScale` of the target it fades out over 140 ms, and it's gone
//!   when it arrives.

/// A projectile.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Projectile {
	pub brush: String,
	from: (f32, f32),
	to: (f32, f32),
	/// Virtual seconds it left at.
	start: f32,
	/// Seconds it flies.
	duration: f32,
	pub scale: f32,
	rotated: bool,
	pub flipped: bool,
	/// Degrees per second it turns (not rotated).
	torque: f32,
	/// The share of the flight after which it fades out.
	fade_from: f32,
}

/// How long it fades out, in seconds.
const FADE_OUT: f32 = 0.14;

/// How long a projectile from `from` to `to` flies, in seconds.
pub(crate) fn flight_time(from: (f32, f32), to: (f32, f32), time_scale: f32) -> f32 {
	let (dx, dy) = (to.0 - from.0, to.1 + 5.0 - from.1);
	let d = f64::from(dy * dy + dx * dx).sqrt() as f32;
	(d * 0.0007).min(0.75) * time_scale
}

impl Projectile {
	pub fn new(brush: String, from: (f32, f32), to: (f32, f32), now: f32, scale: f32, time_scale: f32, rotated: bool, flipped: bool, torque: f32) -> Projectile {
		let mut p = Projectile {
			brush,
			from,
			to,
			start: now,
			duration: flight_time(from, to, time_scale),
			scale,
			rotated,
			flipped,
			torque: if flipped { -torque } else { torque },
			fade_from: 1.0,
		};
		// The first moment it's close enough to fade (the original checks each frame).
		let near = 200.0 / time_scale;
		let steps = 256;
		p.fade_from = (0..=steps)
			.map(|i| i as f32 / steps as f32)
			.find(|&t| {
				let (x, y) = p.pos_at(t);
				let (dx, dy) = (p.to.0 - x, p.to.1 - y);
				f64::from(dy * dy + dx * dx).sqrt() as f32 <= near
			})
			.unwrap_or(1.0);
		p
	}

	/// The share of its flight done at virtual time `now`.
	fn progress(&self, now: f32) -> f32 {
		(now - self.start) / self.duration
	}

	/// Whether it has arrived by virtual time `now`.
	pub fn arrived(&self, now: f32) -> bool {
		!(self.progress(now) < 1.0)
	}

	fn pos_at(&self, t: f32) -> (f32, f32) {
		let (dx, dy) = (self.to.0 - self.from.0, self.to.1 - self.from.1);
		let lift = (dx.abs() * 0.19).min(260.0);
		let arc = 0.25 - (t - 0.5) * (t - 0.5);
		(dx * t + self.from.0, dy * t + self.from.1 + lift * arc * 2.0)
	}

	/// Where it is at virtual time `now`.
	pub fn pos(&self, now: f32) -> (f32, f32) {
		self.pos_at(self.progress(now))
	}

	/// Its rotation in degrees at virtual time `now`.
	pub fn rotation(&self, now: f32) -> f32 {
		if !self.rotated {
			return self.torque * (now - self.start);
		}
		let (x, y) = self.pos(now);
		let deg = (libm::atan2(f64::from(self.to.0 - x), f64::from(self.to.1 - y)) as f32) * 57.295_78;
		-(if deg < 0.0 { deg + 360.0 } else { deg })
	}

	/// Its alpha (0 to 1) at virtual time `now`.
	pub fn alpha(&self, now: f32) -> f32 {
		let fading = (now - self.start) - self.fade_from * self.duration;
		if fading <= 0.0 { 1.0 } else { (1.0 - fading / FADE_OUT).max(0.0) }
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn flies_in_an_arc_and_arrives() {
		let p = Projectile::new("arrow".into(), (0.0, 0.0), (1000.0, 0.0), 10.0, 1.0, 1.0, true, false, 0.0);
		let half = 10.0 + 0.7 * (1000.0f32 * 1000.0 + 25.0).sqrt() * 0.001 / 2.0;
		let (x, y) = p.pos(half);
		assert!((x - 500.0).abs() < 0.5 && (y - 95.0).abs() < 0.5, "at the top of its arc: {x}, {y}");
		assert!(!p.arrived(half) && p.arrived(11.0));
		// Pointing at the target: right, from above it.
		let r = p.rotation(half);
		assert!(r < -90.0 && r > -180.0, "{r}");
	}

	#[test]
	fn fades_near_the_target() {
		let p = Projectile::new("arrow".into(), (0.0, 0.0), (1000.0, 0.0), 0.0, 1.0, 1.0, true, false, 0.0);
		assert_eq!(p.alpha(0.1), 1.0);
		assert!(p.alpha(p.duration * 0.99) < 1.0);
	}
}
