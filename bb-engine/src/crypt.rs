//! XXTEA (Corrected Block TEA), the cipher Battle Brothers applies to its
//! compiled scripts inside the `.dat` archives.
//!
//! Per Adam Milazzo's analysis, the game encrypts compiled bytecode *during the
//! `sq_writeclosure` write stream*: each write of N bytes has its leading
//! `4 * (N / 4)` bytes XXTEA-transformed as one block, but only when `N >= 8`
//! (i.e. ≥ 2 words); shorter runs and the trailing `N % 4` bytes pass through
//! untouched. Decryption mirrors that per-chunk, with no state kept between
//! chunks — so [`decrypt_chunk`] operates on exactly one such read chunk.
//!
//! Words are little-endian (the game writes on x86). Key and constants are the
//! game's, taken from the same analysis.

/// The 128-bit key hardcoded in Battle Brothers.
const KEY: [u32; 4] = [238_473_842, 20_047_425, 14_005, 978_629_342];
const DELTA: u32 = 0x9e37_79b9;

/// The XXTEA mixing function (`MX` in the reference implementation).
#[inline]
fn mx(sum: u32, y: u32, z: u32, p: usize, e: u32) -> u32 {
	let a = ((z >> 5) ^ (y << 2)).wrapping_add((y >> 3) ^ (z << 4));
	let b = (sum ^ y).wrapping_add(KEY[(((p as u32) & 3) ^ e) as usize] ^ z);
	a ^ b
}

/// Encrypt a word array in place (the inverse of [`decrypt_words`]); used to
/// validate the implementation by round-trip. No-op for fewer than 2 words.
fn encrypt_words(v: &mut [u32]) {
	let n = v.len();
	if n < 2 {
		return;
	}
	let rounds = 6 + 52 / n;
	let mut sum: u32 = 0;
	let mut z = v[n - 1];
	for _ in 0..rounds {
		sum = sum.wrapping_add(DELTA);
		let e = (sum >> 2) & 3;
		for p in 0..n - 1 {
			let y = v[p + 1];
			v[p] = v[p].wrapping_add(mx(sum, y, z, p, e));
			z = v[p];
		}
		let y = v[0];
		v[n - 1] = v[n - 1].wrapping_add(mx(sum, y, z, n - 1, e));
		z = v[n - 1];
	}
}

/// Decrypt a word array in place. No-op for fewer than 2 words.
fn decrypt_words(v: &mut [u32]) {
	let n = v.len();
	if n < 2 {
		return;
	}
	let rounds = (6 + 52 / n) as u32;
	let mut sum = rounds.wrapping_mul(DELTA);
	let mut y = v[0];
	for _ in 0..rounds {
		let e = (sum >> 2) & 3;
		for p in (1..n).rev() {
			let z = v[p - 1];
			v[p] = v[p].wrapping_sub(mx(sum, y, z, p, e));
			y = v[p];
		}
		let z = v[n - 1];
		v[0] = v[0].wrapping_sub(mx(sum, y, z, 0, e));
		y = v[0];
		sum = sum.wrapping_sub(DELTA);
	}
}

/// Decrypt one read chunk in place: the leading `4 * (len / 4)` bytes are XXTEA-
/// decrypted when the chunk is at least 8 bytes; anything shorter (and the
/// trailing `len % 4` bytes) is left as-is.
pub fn decrypt_chunk(buf: &mut [u8]) {
	with_words(buf, decrypt_words);
}

/// Encrypt one read chunk in place — the inverse of [`decrypt_chunk`] (for tests
/// and to build fixtures).
pub fn encrypt_chunk(buf: &mut [u8]) {
	with_words(buf, encrypt_words);
}

/// Load the leading whole words of `buf` (little-endian), run `f`, store back.
fn with_words(buf: &mut [u8], f: impl Fn(&mut [u32])) {
	let n = buf.len() / 4;
	if n < 2 {
		return;
	}
	let mut words: Vec<u32> = (0..n)
		.map(|i| u32::from_le_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]]))
		.collect();
	f(&mut words);
	for (i, word) in words.iter().enumerate() {
		buf[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
	}
}

#[cfg(test)]
mod tests {
	use super::{decrypt_chunk, encrypt_chunk};

	#[test]
	fn encrypt_then_decrypt_is_identity() {
		// Several word counts, incl. the 6+52/n rounds boundaries.
		for words in [2usize, 3, 8, 13, 64] {
			let original: Vec<u8> = (0..words * 4)
				.map(|i| (i as u8).wrapping_mul(37).wrapping_add(11))
				.collect();
			let mut buf = original.clone();
			encrypt_chunk(&mut buf);
			assert_ne!(buf, original, "{words} words: encryption changed the bytes");
			decrypt_chunk(&mut buf);
			assert_eq!(buf, original, "{words} words: decrypt(encrypt(x)) == x");
		}
	}

	#[test]
	fn short_and_partial_chunks_pass_through() {
		// < 8 bytes: untouched entirely.
		for len in [0usize, 1, 4, 7] {
			let original: Vec<u8> = (0..len).map(|i| i as u8).collect();
			let mut buf = original.clone();
			encrypt_chunk(&mut buf);
			assert_eq!(buf, original, "{len} bytes pass through");
		}
		// 8+ bytes but not a multiple of 4: trailing bytes untouched.
		let mut buf: Vec<u8> = (0..11).map(|i| i as u8).collect();
		let tail = buf[8..].to_vec();
		encrypt_chunk(&mut buf);
		assert_eq!(&buf[8..], &tail[..], "trailing %4 bytes untouched");
	}
}
