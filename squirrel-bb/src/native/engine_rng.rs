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

	/// A uniform integer in `[min, max]` (`min` if `max <= min`).
	pub(crate) fn range(&self, min: i64, max: i64) -> i64 {
		if max <= min {
			return min;
		}
		let span = (i128::from(max) - i128::from(min)) as u128 + 1;
		let x = u128::from(self.0.borrow_mut().next_u64());
		(i128::from(min) + (x % span) as i128) as i64
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
