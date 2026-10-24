//! BC4 (one unsigned channel): two 8-bit endpoints and a 3-bit index a texel. With the
//! first endpoint above the second, the indices pick among the two and 6 steps between;
//! otherwise among the two, 4 steps between, 0 and 255.

/// The palette a block's endpoints `e0`, `e1` give, exact (a GPU interpolates at least
/// this precisely).
fn palette(e0: u8, e1: u8) -> [f32; 8] {
	let (a, b) = (f32::from(e0), f32::from(e1));
	if e0 > e1 {
		let step = |i: f32| ((7.0 - i) * a + i * b) / 7.0;
		[a, b, step(1.0), step(2.0), step(3.0), step(4.0), step(5.0), step(6.0)]
	} else {
		let step = |i: f32| ((5.0 - i) * a + i * b) / 5.0;
		[a, b, step(1.0), step(2.0), step(3.0), step(4.0), 0.0, 255.0]
	}
}

/// Each value's nearest palette index, and the summed squared error.
fn indices(values: &[u8; 16], palette: &[f32; 8]) -> ([u8; 16], f32) {
	let mut out = [0u8; 16];
	let mut error = 0.0;
	for (index, &v) in out.iter_mut().zip(values) {
		let (best, e) = palette
			.iter()
			.enumerate()
			.map(|(i, &p)| (i, (p - f32::from(v)).powi(2)))
			.fold((0, f32::MAX), |a, b| if b.1 < a.1 { b } else { a });
		*index = best as u8;
		error += e;
	}
	(out, error)
}

fn pack(e0: u8, e1: u8, indices: &[u8; 16]) -> [u8; 8] {
	let bits = indices.iter().enumerate().fold(0u64, |bits, (i, &index)| bits | u64::from(index) << (3 * i));
	let mut out = [0u8; 8];
	out[0] = e0;
	out[1] = e1;
	out[2..].copy_from_slice(&bits.to_le_bytes()[..6]);
	out
}

/// Encode 16 values (row by row) as a BC4 block: the 8-level mode over their range, and
/// the 6-level one over their range without the 0s and 255s where that's closer.
pub fn encode(values: &[u8; 16]) -> [u8; 8] {
	let (lo, hi) = values.iter().fold((u8::MAX, 0), |(lo, hi), &v| (lo.min(v), hi.max(v)));
	if lo == hi {
		return pack(lo, lo, &[0; 16]);
	}
	let (eight, eight_error) = indices(values, &palette(hi, lo));
	let mut best = (pack(hi, lo, &eight), eight_error);
	let inner = values.iter().filter(|&&v| v != 0 && v != 255);
	let (ilo, ihi) = inner.fold((u8::MAX, 0), |(lo, hi), &v| (lo.min(v), hi.max(v)));
	if ilo <= ihi && (lo == 0 || hi == 255) {
		let (six, six_error) = indices(values, &palette(ilo, ihi));
		if six_error < best.1 {
			best = (pack(ilo, ihi, &six), six_error);
		}
	}
	best.0
}

/// A block's 16 values, rounded to 8 bits (the reference decoder).
pub fn decode(block: &[u8; 8]) -> [u8; 16] {
	let palette = palette(block[0], block[1]);
	let mut bits = [0u8; 8];
	bits[..6].copy_from_slice(&block[2..]);
	let bits = u64::from_le_bytes(bits);
	std::array::from_fn(|i| palette[(bits >> (3 * i) & 7) as usize].round() as u8)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_block_comes_back_within_half_a_step() {
		// Within half a step of the 8-level palette, (hi - lo) / 14, plus the rounding; a
		// block with 0s or 255s may take the 6-level one, a fifth of a narrower range.
		let mut seed = 12345u32;
		for _ in 0..2000 {
			let mut next = || {
				seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
				(seed >> 16) as u8
			};
			let base = next();
			let spread = next() / 4;
			let values: [u8; 16] = std::array::from_fn(|_| base.saturating_add(next() % spread.max(1)));
			let back = decode(&encode(&values));
			let (lo, hi) = values.iter().fold((255u8, 0u8), |(l, h), &v| (l.min(v), h.max(v)));
			let steps = if lo == 0 || hi == 255 { 10.0 } else { 14.0 };
			let bound = f32::from(hi - lo) / steps + 0.5;
			for (v, b) in values.iter().zip(back) {
				assert!(f32::from(v.abs_diff(b)) <= bound, "{values:?} → {back:?}");
			}
		}
	}

	#[test]
	fn extremes_and_flat_blocks_are_exact() {
		let flat = [77u8; 16];
		assert_eq!(decode(&encode(&flat)), flat);
		// 0s and 255s beside a narrow cluster: the 6-level mode keeps all three exact.
		let mut mixed = [120u8; 16];
		mixed[0] = 0;
		mixed[5] = 255;
		mixed[9] = 125;
		assert_eq!(decode(&encode(&mixed)), mixed);
	}
}
