//! The random generators behind `Math.rand` / `Math.seedRandom` / `Math.seedRandomString`,
//! matching the ORIGINAL engine bit-for-bit (reverse-engineered from sequences captured
//! in the original game):
//!
//! * `rand(min, max)` draws from one global [`Mt19937_64`]: `min + X % (max - min + 1)`.
//!   An empty range (`max <= min`) returns `min` **without** consuming a draw.
//! * `seedRandom(x)` truncates `x` toward zero and reinterprets it as a **u32**
//!   (`-1` → `0xFFFFFFFF`), then reseeds the twister with it.
//! * `seedRandomString(s)` reseeds with the 64-bit x65599 (sdbm) hash of its bytes.
//! * `rand()` (no args) is the C runtime's `rand()` — the MSVC LCG — a **separate**
//!   stream that the seed functions never touch.
//!
//! One [`Prng`] is created by the native installer and shared with `color.varyRGB`.

use std::cell::RefCell;
use std::rc::Rc;

use super::mt64::Mt19937_64;

/// The MSVC CRT `rand()` LCG (`state * 214013 + 2531011`, bits 16..30).
struct CrtRand {
	state: u32,
}

impl CrtRand {
	fn next(&mut self) -> i64 {
		self.state = self.state.wrapping_mul(214_013).wrapping_add(2_531_011);
		i64::from((self.state >> 16) & 0x7FFF)
	}
}

struct State {
	mt: Mt19937_64,
	crt: CrtRand,
}

/// Shared RNG state (cheap to clone; all clones draw from the same streams).
#[derive(Clone)]
pub(crate) struct Prng {
	state: Rc<RefCell<State>>,
}

impl Prng {
	/// A new generator. The twister starts at the C++ default seed (5489) and the C
	/// runtime stream at its default (`srand(1)`); the game seeds explicitly wherever
	/// determinism matters.
	pub(crate) fn new() -> Self {
		Prng {
			state: Rc::new(RefCell::new(State {
				mt: Mt19937_64::new(5489),
				crt: CrtRand { state: 1 },
			})),
		}
	}

	/// `seedRandom`: the (already truncated) integer is reinterpreted as a u32.
	pub(crate) fn seed(&self, seed: i64) {
		self.state.borrow_mut().mt = Mt19937_64::new(u64::from(seed as u32));
	}

	/// `seedRandomString`: x65599 hash (`h = h * 65599 + byte`) over the bytes, 64-bit.
	pub(crate) fn seed_str(&self, text: &str) {
		let hash = text
			.as_bytes()
			.iter()
			.fold(0u64, |h, &b| h.wrapping_mul(65_599).wrapping_add(u64::from(b)));
		self.state.borrow_mut().mt = Mt19937_64::new(hash);
	}

	/// `rand()`: the separate C-runtime stream, in `[0, 32767]`.
	pub(crate) fn rand_default(&self) -> i64 {
		self.state.borrow_mut().crt.next()
	}

	/// `rand(min, max)`: inclusive; `min` (and no draw) when `max <= min`.
	///
	/// Computed in `i128`/`u128` so no `i64` inputs can overflow — a panic here would
	/// unwind across the C trampoline.
	pub(crate) fn rand_range(&self, min: i64, max: i64) -> i64 {
		if max <= min {
			return min;
		}
		let span = (i128::from(max) - i128::from(min)) as u128 + 1;
		let x = u128::from(self.state.borrow_mut().mt.next_u64());
		(i128::from(min) + (x % span) as i128) as i64
	}

	/// A uniform float in `[lo, hi)` from the twister (returns `lo` if `hi <= lo`).
	/// Backs `color.varyRGB`. NOTE: the original's exact float derivation for varyRGB
	/// is not yet verified; this uses the top 53 bits of one draw.
	pub(crate) fn rand_range_f(&self, lo: f64, hi: f64) -> f64 {
		if hi <= lo {
			return lo;
		}
		let x = self.state.borrow_mut().mt.next_u64();
		lo + ((x >> 11) as f64 / (1u64 << 53) as f64) * (hi - lo)
	}
}
