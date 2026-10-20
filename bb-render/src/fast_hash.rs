//! A fast hasher for the per-quad maps a frame builds (the multiply-rotate scheme of
//! rustc's FxHash). Not DoS-resistant, which keys from our own scene don't need; std's
//! SipHash cost more per quad than the rest of a quad's work.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// The hasher's state.
#[derive(Clone, Copy, Default)]
pub(crate) struct FastHasher(u64);

const K: u64 = 0xf135_7aea_2e62_a9c5;

impl FastHasher {
	#[inline]
	fn add(&mut self, word: u64) {
		self.0 = (self.0.rotate_left(26) ^ word).wrapping_mul(K);
	}
}

impl Hasher for FastHasher {
	#[inline]
	fn write(&mut self, bytes: &[u8]) {
		let mut chunks = bytes.chunks_exact(8);
		for chunk in &mut chunks {
			self.add(u64::from_le_bytes(chunk.try_into().expect("8 bytes")));
		}
		let rest = chunks.remainder();
		if !rest.is_empty() {
			let mut last = [0u8; 8];
			last[..rest.len()].copy_from_slice(rest);
			self.add(u64::from_le_bytes(last));
		}
	}

	#[inline]
	fn write_u8(&mut self, n: u8) {
		self.add(u64::from(n));
	}

	#[inline]
	fn write_u32(&mut self, n: u32) {
		self.add(u64::from(n));
	}

	#[inline]
	fn write_u64(&mut self, n: u64) {
		self.add(n);
	}

	#[inline]
	fn write_usize(&mut self, n: usize) {
		self.add(n as u64);
	}

	#[inline]
	fn finish(&self) -> u64 {
		self.0
	}
}

pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn keys_stay_apart() {
		let mut map: FastMap<(usize, [u32; 8]), usize> = FastMap::default();
		for i in 0..1000 {
			map.insert((i % 7, [i as u32, 0, 1, 2, 3, 4, 5, (i * 3) as u32]), i);
		}
		assert_eq!(map.len(), 1000);
		assert_eq!(map[&(3, [10, 0, 1, 2, 3, 4, 5, 30])], 10);
		let mut paths: FastMap<&str, ()> = FastMap::default();
		for path in ["gfx/a.png", "gfx/b.png", "gfx/a.png"] {
			paths.insert(path, ());
		}
		assert_eq!(paths.len(), 2);
	}
}
