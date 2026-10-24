//! BC1 (opaque colour): two RGB565 endpoints and a 2-bit index a texel. With the first
//! endpoint above the second (as 16-bit numbers), the indices pick among the two and
//! the colours a third and two thirds between; the encoder only makes such blocks (or
//! ones of a single colour).

type Rgb = [f32; 3];

/// An RGB565 colour as 8-bit channels (each widened by repeating its top bits).
fn expand(c: u16) -> Rgb {
	let (r, g, b) = (c >> 11, (c >> 5) & 63, c & 31);
	[(r << 3 | r >> 2) as f32, (g << 2 | g >> 4) as f32, (b << 3 | b >> 2) as f32]
}

/// `c` (8-bit channels, unrounded) to the nearest RGB565.
fn quantize(c: Rgb) -> u16 {
	let q = |v: f32, max: f32| (v.clamp(0.0, 255.0) * max / 255.0).round() as u16;
	q(c[0], 31.0) << 11 | q(c[1], 63.0) << 5 | q(c[2], 31.0)
}

/// The four colours of endpoints `c0` > `c1`, exact (a GPU interpolates within the
/// format's tolerance of these).
fn palette(c0: u16, c1: u16) -> [Rgb; 4] {
	let (a, b) = (expand(c0), expand(c1));
	let mix = |wa: f32| std::array::from_fn(|i| (wa * a[i] + (3.0 - wa) * b[i]) / 3.0);
	[a, b, mix(2.0), mix(1.0)]
}

fn distance(a: &Rgb, b: &Rgb) -> f32 {
	(0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

/// How far each palette index lies from the first endpoint towards the second.
const WEIGHT: [f32; 4] = [0.0, 1.0, 1.0 / 3.0, 2.0 / 3.0];

/// The block for endpoints `c0`, `c1` (put in order), its indices and summed squared
/// error over `texels`.
fn fit(texels: &[Rgb; 16], c0: u16, c1: u16) -> ([u8; 8], [u8; 16], f32) {
	let (c0, c1) = if c0 >= c1 { (c0, c1) } else { (c1, c0) };
	let mut indices = [0u8; 16];
	let mut error = 0.0;
	if c0 == c1 {
		let colour = expand(c0);
		error = texels.iter().map(|t| distance(t, &colour)).sum();
	} else {
		let palette = palette(c0, c1);
		for (index, texel) in indices.iter_mut().zip(texels) {
			let (best, e) = (0..4)
				.map(|i| (i, distance(texel, &palette[i])))
				.fold((0, f32::MAX), |a, b| if b.1 < a.1 { b } else { a });
			*index = best as u8;
			error += e;
		}
	}
	let bits = indices.iter().enumerate().fold(0u32, |bits, (i, &index)| bits | u32::from(index) << (2 * i));
	let mut block = [0u8; 8];
	block[..2].copy_from_slice(&c0.to_le_bytes());
	block[2..4].copy_from_slice(&c1.to_le_bytes());
	block[4..].copy_from_slice(&bits.to_le_bytes());
	(block, indices, error)
}

/// The endpoints that best fit `texels` (least squares) given each one's index; `None`
/// where the indices don't tell them apart (all at one end).
fn refit(texels: &[Rgb; 16], indices: &[u8; 16]) -> Option<(Rgb, Rgb)> {
	let (mut aa, mut ab, mut bb) = (0.0, 0.0, 0.0);
	let (mut ap, mut bp) = ([0.0f32; 3], [0.0f32; 3]);
	for (texel, &index) in texels.iter().zip(indices) {
		let b = WEIGHT[index as usize];
		let a = 1.0 - b;
		aa += a * a;
		ab += a * b;
		bb += b * b;
		for i in 0..3 {
			ap[i] += a * texel[i];
			bp[i] += b * texel[i];
		}
	}
	let det = aa * bb - ab * ab;
	if det.abs() < 1e-6 {
		return None;
	}
	let e0 = std::array::from_fn(|i| (bb * ap[i] - ab * bp[i]) / det);
	let e1 = std::array::from_fn(|i| (aa * bp[i] - ab * ap[i]) / det);
	Some((e0, e1))
}

/// The colours' principal axis (unit length, or zero for a flat block) and their mean.
fn principal_axis(texels: &[Rgb; 16]) -> (Rgb, Rgb) {
	let mean: Rgb = std::array::from_fn(|i| texels.iter().map(|t| t[i]).sum::<f32>() / 16.0);
	let mut cov = [[0.0f32; 3]; 3];
	for t in texels {
		let d: Rgb = std::array::from_fn(|i| t[i] - mean[i]);
		for i in 0..3 {
			for j in 0..3 {
				cov[i][j] += d[i] * d[j];
			}
		}
	}
	// Power iteration, from the channel that varies most.
	let start = (0..3).max_by(|&a, &b| cov[a][a].total_cmp(&cov[b][b])).unwrap_or(0);
	let mut axis = [0.0f32; 3];
	axis[start] = 1.0;
	for _ in 0..8 {
		let next: Rgb = std::array::from_fn(|i| (0..3).map(|j| cov[i][j] * axis[j]).sum());
		let length = next.iter().map(|v| v * v).sum::<f32>().sqrt();
		if length < 1e-6 {
			return ([0.0; 3], mean);
		}
		axis = next.map(|v| v / length);
	}
	(axis, mean)
}

/// Encode 16 RGB texels (row by row) as a BC1 block.
pub fn encode(texels: &[[u8; 3]; 16]) -> [u8; 8] {
	let texels: [Rgb; 16] = texels.map(|t| t.map(f32::from));
	let (axis, mean) = principal_axis(&texels);
	let along = |t: &Rgb| (0..3).map(|i| (t[i] - mean[i]) * axis[i]).sum::<f32>();
	let (lo, hi) = texels.iter().map(along).fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(d), hi.max(d)));
	let at = |d: f32| -> Rgb { std::array::from_fn(|i| mean[i] + axis[i] * d) };
	let (mut best, mut best_indices, mut best_error) = fit(&texels, quantize(at(hi)), quantize(at(lo)));
	// The bounding box's corners too: a better start where the axis misleads.
	let corner = |pick: fn(f32, f32) -> f32| -> Rgb {
		std::array::from_fn(|i| texels.iter().map(|t| t[i]).fold(texels[0][i], pick))
	};
	let (block, indices, error) = fit(&texels, quantize(corner(f32::max)), quantize(corner(f32::min)));
	if error < best_error {
		(best, best_indices, best_error) = (block, indices, error);
	}
	for _ in 0..3 {
		if best_error == 0.0 {
			break;
		}
		// The refit works on the order the block was written in.
		let Some((r0, r1)) = refit(&texels, &best_indices) else { break };
		let (block, indices, error) = fit(&texels, quantize(r0), quantize(r1));
		if error >= best_error {
			break;
		}
		(best, best_indices, best_error) = (block, indices, error);
	}
	// Then each endpoint's channels a step up or down while that helps: the 565
	// rounding of the fitted endpoints isn't the best pair of 565 colours.
	for _ in 0..ROUNDS {
		if best_error == 0.0 {
			break;
		}
		let ends = [u16::from_le_bytes([best[0], best[1]]), u16::from_le_bytes([best[2], best[3]])];
		let mut improved = false;
		for (end, (shift, bits), up) in (0..2).flat_map(|e| CHANNELS.into_iter().flat_map(move |c| [(e, c, true), (e, c, false)])) {
			let Some(moved) = step(ends[end], shift, bits, up) else { continue };
			let mut pair = ends;
			pair[end] = moved;
			let (block, _, error) = fit(&texels, pair[0], pair[1]);
			if error < best_error {
				(best, best_error) = (block, error);
				improved = true;
				break;
			}
		}
		if !improved {
			break;
		}
	}
	best
}

/// At most how many steps the endpoint search takes.
const ROUNDS: usize = 8;

/// An RGB565 colour's channels: their shift and width.
const CHANNELS: [(u16, u16); 3] = [(11, 5), (5, 6), (0, 5)];

/// `colour` with the channel at `shift` (`bits` wide) a step up or down; `None` past
/// its range.
fn step(colour: u16, shift: u16, bits: u16, up: bool) -> Option<u16> {
	let max = (1 << bits) - 1;
	let value = (colour >> shift) & max;
	let moved = if up { value.checked_add(1).filter(|v| *v <= max)? } else { value.checked_sub(1)? };
	Some(colour & !(max << shift) | moved << shift)
}

/// A block's 16 colours as RGBA, rounded to 8 bits (the reference decoder). A block of
/// the 3-colour mode (first endpoint not above the second) decodes as the format says:
/// the third colour halfway, the fourth black.
pub fn decode(block: &[u8; 8]) -> [[u8; 4]; 16] {
	let c0 = u16::from_le_bytes([block[0], block[1]]);
	let c1 = u16::from_le_bytes([block[2], block[3]]);
	let palette = if c0 > c1 {
		palette(c0, c1)
	} else {
		let (a, b) = (expand(c0), expand(c1));
		[a, b, std::array::from_fn(|i| (a[i] + b[i]) / 2.0), [0.0; 3]]
	};
	let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
	std::array::from_fn(|i| {
		let c = palette[(bits >> (2 * i) & 3) as usize];
		[c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, 255]
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_flat_block_comes_back_at_least_as_close_as_its_nearest_565_colour() {
		for colour in [[0u8, 0, 0], [255, 255, 255], [26, 48, 46], [133, 133, 133]] {
			let back = decode(&encode(&[colour; 16]));
			let nearest = expand(quantize(colour.map(f32::from)));
			let error = |c: &[f32]| (0..3).map(|i| (c[i] - f32::from(colour[i])).powi(2)).sum::<f32>();
			for texel in back {
				// A colour between two 565 ones can come closer than either.
				assert!(error(&texel[..3].iter().map(|&v| f32::from(v)).collect::<Vec<_>>()) <= error(&nearest) + 1.0, "{colour:?} → {texel:?}");
			}
		}
	}

	#[test]
	fn a_two_colour_block_is_exact_at_565_colours() {
		// 565 colours: 31, 63, 0 and 10, 20, 31 widened.
		let (a, b) = ([255u8, 255, 0], [82u8, 81, 255]);
		let texels: [[u8; 3]; 16] = std::array::from_fn(|i| if i % 3 == 0 { a } else { b });
		let back = decode(&encode(&texels));
		for (t, got) in texels.iter().zip(back) {
			assert_eq!(&got[..3], &t[..], "two 565 colours come back exactly");
		}
	}

	#[test]
	fn a_gradient_comes_back_within_a_few_levels() {
		// A block along a line through colour space: the format's best case, each texel
		// within the 565 rounding (4 levels in red and blue, 2 in green) and a little.
		let texels: [[u8; 3]; 16] = std::array::from_fn(|i| {
			let t = i as f32 / 15.0;
			[(20.0 + 14.0 * t) as u8, (40.0 + 24.0 * t) as u8, (38.0 + 22.0 * t) as u8]
		});
		let back = decode(&encode(&texels));
		for (t, got) in texels.iter().zip(back) {
			for c in 0..3 {
				assert!(got[c].abs_diff(t[c]) <= 5, "{t:?} → {got:?}");
			}
		}
	}

	#[test]
	fn the_encoder_only_writes_four_colour_or_flat_blocks() {
		let mut seed = 99u32;
		for _ in 0..500 {
			let texels: [[u8; 3]; 16] = std::array::from_fn(|_| {
				std::array::from_fn(|_| {
					seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
					(seed >> 16) as u8
				})
			});
			let block = encode(&texels);
			let (c0, c1) = (u16::from_le_bytes([block[0], block[1]]), u16::from_le_bytes([block[2], block[3]]));
			let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
			assert!(c0 > c1 || bits == 0, "{c0} {c1} {bits:x}");
		}
	}
}
