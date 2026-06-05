//! The pseudo-random generator behind `Math.rand` / `Math.seedRandom`.
//!
//! Battle Brothers' exact RNG algorithm is unknown (it would only matter for
//! reproducing a specific campaign seed or save, both out of scope), so this is a
//! deliberately self-contained, deterministic stand-in: SplitMix64. It is
//! isolated here precisely so the algorithm can be swapped later without touching
//! the `Math` table. The two-arg gameplay RNG and the seed functions all share
//! one state, matching the observable BB contract (`seedRandom(x)` then `rand(...)`).

use std::cell::Cell;
use std::rc::Rc;

/// Shared, mutable RNG state (a single 64-bit SplitMix64 register).
#[derive(Clone)]
pub(crate) struct Prng {
	state: Rc<Cell<u64>>,
}

impl Prng {
	/// A new generator with a fixed, deterministic default seed.
	pub(crate) fn new() -> Self {
		Prng {
			state: Rc::new(Cell::new(0x2545_F491_4F6C_DD1D)),
		}
	}

	/// Re-seed from an integer.
	pub(crate) fn seed(&self, seed: i64) {
		self.state.set(seed as u64);
	}

	/// Re-seed from a string (FNV-1a 64-bit of its bytes).
	pub(crate) fn seed_str(&self, text: &str) {
		const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
		const PRIME: u64 = 0x0000_0100_0000_01b3;
		let mut hash = OFFSET;
		for byte in text.as_bytes() {
			hash ^= u64::from(*byte);
			hash = hash.wrapping_mul(PRIME);
		}
		self.state.set(hash);
	}

	/// Next raw 64 bits (advances the state); SplitMix64.
	fn next_u64(&self) -> u64 {
		let mut z = self.state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
		self.state.set(z);
		z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
		z ^ (z >> 31)
	}

	/// An integer in `[0, 32767]` (the BB zero-arg `Math.rand()` range).
	pub(crate) fn rand_default(&self) -> i64 {
		(self.next_u64() % 32768) as i64
	}

	/// A uniform float in `[0, 1)`, using the top 53 bits (exact in an `f64`).
	fn next_unit(&self) -> f64 {
		(self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
	}

	/// A uniform float in `[lo, hi)` (returns `lo` if `hi <= lo`). Backs
	/// `color.varyRGB`, whose channel factors draw from `[1-f, 1+f]`.
	pub(crate) fn rand_range_f(&self, lo: f64, hi: f64) -> f64 {
		if hi <= lo {
			return lo;
		}
		lo + self.next_unit() * (hi - lo)
	}

	/// An integer in `[min, max]` inclusive. If `max <= min`, returns `min`.
	///
	/// Computed in `i128` so it cannot overflow or divide by zero for *any* `i64`
	/// inputs — a panic here would unwind across the C trampoline (UB). Vanilla
	/// only ever passes small ranges, but a script/mod could pass extremes.
	pub(crate) fn rand_range(&self, min: i64, max: i64) -> i64 {
		if max <= min {
			return min;
		}
		let span = (max as i128 - min as i128) as u128 + 1;
		let offset = (u128::from(self.next_u64()) % span) as i128;
		(min as i128 + offset) as i64
	}
}
