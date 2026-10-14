//! [`EngineRng`]: the engine's own random numbers, separate from the scripts'
//! `Math.rand` stream.
//!
//! The original keeps a `std::mt19937_64` at `0x69f318`, seeded once at startup from
//! `std::random_device` (`0x480e94`, OS entropy), and draws from it with
//! `uniform_int_distribution` for engine-side variety, e.g. which world details are
//! mirrored and their random scale and rotation (`spawnDetail`, `0x4fb5d0`). Those
//! choices differ between any two runs of the original and can't be recovered; they
//! never touch the script stream, which is why seed-exact worldgen doesn't depend on
//! them.
//!
//! We draw with `min + x % span` rather than MSVC's `_Rng_from_urng`. The spread is
//! the same, and no run of the original can be reproduced either way.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::mt64::Mt19937_64;

/// The engine's random stream (shared by the natives that draw from it).
#[derive(Clone)]
pub struct EngineRng(Rc<RefCell<Mt19937_64>>);

impl EngineRng {
	/// Unpredictably seeded (from the clock here; the original uses OS entropy).
	pub fn unseeded() -> Self {
		let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
		Self::seeded(nanos as u64)
	}

	/// Seeded with `seed`, for reproducible runs (tests, comparisons).
	pub fn seeded(seed: u64) -> Self {
		EngineRng(Rc::new(RefCell::new(Mt19937_64::new(seed))))
	}

	/// Reseed in place (every holder shares the stream).
	pub fn reseed(&self, seed: u64) {
		*self.0.borrow_mut() = Mt19937_64::new(seed);
	}

	/// A uniform float in `[0, 1)`.
	pub(crate) fn unit(&self) -> f32 {
		((self.0.borrow_mut().next_u64() >> 40) as f32) / (1u64 << 24) as f32
	}

	/// A uniform float in `[min, max)`, or `min` without drawing when `min >= max`
	/// (the original's cloud and weather draws skip the RNG then).
	pub(crate) fn between(&self, min: f32, max: f32) -> f32 {
		if min < max { min + self.unit() * (max - min) } else { min }
	}

	/// A uniform integer in `[min, max]` (`min` if `max <= min`).
	pub(crate) fn range(&self, min: i64, max: i64) -> i64 {
		if max <= min {
			return min;
		}
		let span = (i128::from(max) - i128::from(min)) as u128 + 1;
		let x = u128::from(self.0.borrow_mut().next_u64());
		(i128::from(min) + (x % span) as i128) as i64
	}

	/// An index in `[0, n)` as MSVC's `uniform_int_distribution` draws it
	/// (`_Rng_from_urng`, `0x4b0b80`): a 64-bit draw, drawn again while it lands in the
	/// top partial bucket, modulo `n`. `n` must be at least 1.
	pub(crate) fn uniform_index(&self, n: u64) -> u64 {
		loop {
			let x = self.0.borrow_mut().next_u64();
			if x / n != u64::MAX / n || u64::MAX % n == n - 1 {
				return x % n;
			}
		}
	}
}

impl Default for EngineRng {
	fn default() -> Self {
		Self::unseeded()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_same_seed_gives_the_same_draws() {
		let (a, b) = (EngineRng::seeded(7), EngineRng::seeded(7));
		let draws = |r: &EngineRng| (0..20).map(|_| r.range(0, 359)).collect::<Vec<_>>();
		assert_eq!(draws(&a), draws(&b));
	}

	#[test]
	fn draws_stay_in_range_and_cover_it() {
		let r = EngineRng::seeded(1);
		let draws: Vec<i64> = (0..200).map(|_| r.range(0, 1)).collect();
		assert!(draws.iter().all(|&d| d == 0 || d == 1));
		assert!(draws.contains(&0) && draws.contains(&1));
		assert_eq!(r.range(5, 5), 5);
	}
}
