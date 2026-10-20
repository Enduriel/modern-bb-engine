//! A pool on the ground (`spawnPoolEffect`: blood under the hit and the dead; the
//! original's `0x4bc620`, its update `0x4bc8b0`; disassembled 2026-10-14). It stays
//! for the battle. Its looks are random, from the engine's stream, drawn in this
//! order: how far it spreads (15–25), a sideways shift (−8–8), its size (0.4–0.8 of
//! the script's scale), its rotation (90–270°), a mirror; then the C runtime's
//! `rand()` varies its colour (`0x4bc4b0`: each channel `0.8 c + 0.2 r / 255`).
//! Over its first 6 s of virtual time it spreads along its rotation, eased
//! (`min(1, max(0.25, (2 − p) p))` of the way), growing by up to 0.2.

use crate::native::EngineRng;

/// A pool.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pool {
	pub tile: usize,
	pub brush: String,
	/// From the tile, before it spreads.
	offset: (f32, f32),
	spread: f32,
	size: f32,
	/// Degrees.
	pub rotation: f32,
	pub mirrored: bool,
	/// ARGB.
	pub color: u32,
	/// Virtual seconds it was spilled at.
	start: f32,
	/// The script's detail flags.
	pub flags: i64,
}

/// How long a pool spreads, in seconds.
const SPREADS_FOR: f32 = 6.0;

impl Pool {
	/// A pool of `brush` on `tile` at virtual time `now`, `alpha` (0 to 1) and `scale`
	/// as the script asks; `crt` is the C runtime's next `rand()`.
	pub fn new(rng: &EngineRng, crt: i64, tile: usize, brush: String, alpha: f32, scale: f32, flags: i64, now: f32) -> Pool {
		let spread = rng.unit() * 10.0 + 15.0;
		let dx = rng.unit() * 16.0 - 8.0;
		let size = (rng.unit() * 0.4 + 0.4) * scale;
		let rotation = rng.unit() * 180.0 + 90.0;
		let mirrored = rng.range(0, 1) == 0;
		let k = (crt & 0xff) as f32 * (1.0 / 255.0) * 0.2;
		let channel = |c: f32| ((0.8 * c + k) * 255.0) as u32;
		let (r, g, b) = (channel(1.0), channel(1.0), channel(1.0));
		let a = (alpha * 255.0) as u32 & 0xff;
		Pool {
			tile,
			brush,
			offset: (dx, -0.0),
			spread,
			size,
			rotation,
			mirrored,
			color: a << 24 | r << 16 | g << 8 | b,
			start: now,
			flags,
		}
	}

	/// How far it has spread at virtual time `now`, 0.25 to 1.
	fn eased(&self, now: f32) -> f32 {
		let p = ((now - self.start) / SPREADS_FOR).min(1.0);
		((2.0 - p) * p).max(0.25).min(1.0)
	}

	/// Where it is from its tile at virtual time `now`.
	pub fn offset(&self, now: f32) -> (f32, f32) {
		let e = self.eased(now);
		let angle = f64::from(self.rotation * 0.017_453_292);
		let (sin, cos) = (libm::sin(angle) as f32, libm::cos(angle) as f32);
		(self.offset.0 + self.spread * -sin * e, self.offset.1 + self.spread * cos * e)
	}

	/// Its scale at virtual time `now`.
	pub fn scale(&self, now: f32) -> f32 {
		self.eased(now) * 0.2 + self.size
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn spreads_then_rests() {
		let rng = EngineRng::seeded(7);
		let pool = Pool::new(&rng, 0x1ff, 3, "blood".into(), 0.5, 1.0, 0, 100.0);
		assert_eq!(pool.color >> 24, 127);
		assert_eq!(pool.color & 0xff, ((0.8 + 1.0 * 0.2) * 255.0) as u32, "the low byte, 255, brightens it fully");
		assert_eq!(pool.scale(100.0), pool.size + 0.05, "it starts a quarter spread");
		assert_eq!(pool.scale(106.0), pool.size + 0.2);
		assert_eq!(pool.offset(200.0), pool.offset(106.0), "then it rests");
	}
}
