//! The original engine's 64-bit string hash, which keys its sprite registry.
//!
//! It is x65599 (sdbm): `h = h * 65599 + c` over the bytes, wrapping at 64 bits.
//! Each byte is read as a **signed** `char` and sign-extended, so bytes from 0x80 up
//! add a negative value (disassembly: `movsx` then `cdq` in the hash loop at
//! `0x4a3350`). Every sprite in a `.brush` file stores this hash of its own name, and
//! `doesBrushExist` hashes its argument the same way.

/// The original's x65599 hash of `bytes`, each byte taken as a signed `char`.
pub fn name_hash(bytes: &[u8]) -> u64 {
	bytes.iter().fold(0u64, |hash, &byte| {
		hash.wrapping_mul(65_599)
			.wrapping_add(i64::from(byte as i8) as u64)
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn matches_a_hash_stored_in_a_vanilla_brush() {
		// `debug.brush`'s only sprite stores `0e bf fd f8 64 55 50 52` before its name.
		assert_eq!(name_hash(b"debug_circle_100"), 0x5250_5564_f8fd_bf0e);
	}

	#[test]
	fn empty_input_hashes_to_zero() {
		assert_eq!(name_hash(b""), 0);
	}

	#[test]
	fn high_bytes_are_sign_extended() {
		assert_eq!(name_hash(&[0x80]), (-128i64) as u64);
		assert_eq!(
			name_hash(&[b'a', 0xff]),
			97u64.wrapping_mul(65_599).wrapping_sub(1)
		);
	}
}
