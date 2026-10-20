//! A fast hasher for the profiler's pointer keys (the std SipHash costs more than the
//! rest of an event). Not DoS resistant; keys are addresses, never input.

use std::hash::{BuildHasherDefault, Hasher};

/// FxHash's multiply-rotate mix.
#[derive(Default)]
pub(super) struct PtrHasher(u64);

impl Hasher for PtrHasher {
	fn write(&mut self, bytes: &[u8]) {
		for &b in bytes {
			self.write_u64(b as u64);
		}
	}

	fn write_u64(&mut self, n: u64) {
		self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
	}

	fn write_u32(&mut self, n: u32) {
		self.write_u64(n as u64);
	}

	fn write_i64(&mut self, n: i64) {
		self.write_u64(n as u64);
	}

	fn write_usize(&mut self, n: usize) {
		self.write_u64(n as u64);
	}

	fn finish(&self) -> u64 {
		self.0
	}
}

pub(super) type PtrMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<PtrHasher>>;
