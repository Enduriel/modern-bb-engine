//! MT19937-64 — the 64-bit Mersenne Twister, bit-identical to C++ `std::mt19937_64`.
//!
//! The original engine's seeded `Math.rand(min, max)` draws from a `std::mt19937_64`
//! (verified against sequences captured from the original game).

const N: usize = 312;
const M: usize = 156;
const MATRIX_A: u64 = 0xB502_6F5A_A966_19E9;
const UPPER_MASK: u64 = 0xFFFF_FFFF_8000_0000;
const LOWER_MASK: u64 = 0x7FFF_FFFF;

pub(crate) struct Mt19937_64 {
	state: [u64; N],
	index: usize,
}

impl Mt19937_64 {
	/// Seed exactly like `std::mt19937_64(seed)` / `.seed(seed)`.
	pub(crate) fn new(seed: u64) -> Self {
		let mut state = [0u64; N];
		state[0] = seed;
		for i in 1..N {
			let prev = state[i - 1];
			state[i] = 6_364_136_223_846_793_005u64
				.wrapping_mul(prev ^ (prev >> 62))
				.wrapping_add(i as u64);
		}
		Mt19937_64 { state, index: N }
	}

	pub(crate) fn next_u64(&mut self) -> u64 {
		if self.index >= N {
			self.twist();
		}
		let mut x = self.state[self.index];
		self.index += 1;
		x ^= (x >> 29) & 0x5555_5555_5555_5555;
		x ^= (x << 17) & 0x71D6_7FFF_EDA6_0000;
		x ^= (x << 37) & 0xFFF7_EEE0_0000_0000;
		x ^ (x >> 43)
	}

	fn twist(&mut self) {
		for i in 0..N {
			let x = (self.state[i] & UPPER_MASK) | (self.state[(i + 1) % N] & LOWER_MASK);
			let mut xa = x >> 1;
			if x & 1 != 0 {
				xa ^= MATRIX_A;
			}
			self.state[i] = self.state[(i + M) % N] ^ xa;
		}
		self.index = 0;
	}
}

#[cfg(test)]
mod tests {
	use super::Mt19937_64;

	#[test]
	fn matches_cpp_reference_value() {
		// The C++ standard requires the 10000th draw of a default-seeded (5489)
		// mt19937_64 to be 9981545732273789042 ([rand.predef]).
		let mut mt = Mt19937_64::new(5489);
		let mut v = 0;
		for _ in 0..10_000 {
			v = mt.next_u64();
		}
		assert_eq!(v, 9_981_545_732_273_789_042);
	}
}
