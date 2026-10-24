//! Block compression on the CPU: BC1 (colour, 4 bits a texel) and BC4 (one channel, 4
//! bits a texel), for the world map's water and fog textures (`crate::world_textures`).
//! Each 4×4 block is encoded on its own; an image is cut into blocks row by row, edges
//! past an image's side repeating its last texel (a mip level under 4 texels wide).
//!
//! The encoders are simple ones (the colours' principal axis, then a least-squares
//! refit; the extremes for BC4), which suit these textures' smooth, low-contrast
//! content. Each has a reference decoder, for the tests' error bounds: a GPU decodes
//! within the formats' tolerance of it.

pub mod bc1;
pub mod bc4;

use bb_assets::Image;

/// The bytes of one block of either format.
pub const BLOCK_BYTES: usize = 8;

/// How many 4×4 blocks cover `width` × `height` texels, across and down.
pub fn blocks(width: u32, height: u32) -> (u32, u32) {
	(width.div_ceil(4), height.div_ceil(4))
}

/// Texels to encode: `width` × `height`, row by row, `stride` bytes apart, the channels
/// wanted first in each.
#[derive(Clone, Copy)]
pub struct Plane<'a> {
	pub width: u32,
	pub height: u32,
	pub bytes: &'a [u8],
	pub stride: usize,
}

impl<'a> Plane<'a> {
	/// An RGBA image's texels (its red first).
	pub fn rgba(image: &'a Image) -> Plane<'a> {
		Plane { width: image.width, height: image.height, bytes: &image.rgba, stride: 4 }
	}

	/// The 16 texels of block (`bx`, `by`), row by row, their first `N` channels each;
	/// texels past an edge repeat the edge's.
	fn block<const N: usize>(&self, bx: u32, by: u32) -> [[u8; N]; 16] {
		std::array::from_fn(|i| {
			let x = (bx * 4 + i as u32 % 4).min(self.width - 1);
			let y = (by * 4 + i as u32 / 4).min(self.height - 1);
			let at = (y * self.width + x) as usize * self.stride;
			std::array::from_fn(|c| self.bytes[at + c])
		})
	}
}

/// `plane`'s blocks, row by row, each made by `encode` from the block's texels: on up
/// to `threads` threads (rows of blocks split between them), the same bytes as on one.
fn encode_plane<const N: usize>(plane: &Plane, threads: usize, encode: fn(&[[u8; N]; 16]) -> [u8; 8]) -> Vec<u8> {
	let (across, down) = blocks(plane.width, plane.height);
	let row_bytes = across as usize * BLOCK_BYTES;
	let mut out = vec![0u8; row_bytes * down as usize];
	let rows_each = (down as usize).div_ceil(threads.max(1)).max(1);
	std::thread::scope(|scope| {
		for (part, chunk) in out.chunks_mut(rows_each * row_bytes).enumerate() {
			scope.spawn(move || {
				for (row, bytes) in chunk.chunks_mut(row_bytes).enumerate() {
					let by = (part * rows_each + row) as u32;
					for bx in 0..across {
						let at = bx as usize * BLOCK_BYTES;
						bytes[at..at + BLOCK_BYTES].copy_from_slice(&encode(&plane.block::<N>(bx, by)));
					}
				}
			});
		}
	});
	out
}

/// `plane`'s first three channels as opaque BC1 blocks, row by row.
pub fn encode_bc1(plane: &Plane, threads: usize) -> Vec<u8> {
	encode_plane::<3>(plane, threads, bc1::encode)
}

/// `plane`'s first channel as BC4 blocks, row by row.
pub fn encode_bc4(plane: &Plane, threads: usize) -> Vec<u8> {
	encode_plane::<1>(plane, threads, |texels| bc4::encode(&texels.map(|t| t[0])))
}

/// Decode `blocks` (of a `width` × `height` image, row by row) with `decode`, into RGBA.
fn decode_image(width: u32, height: u32, blocks_bytes: &[u8], decode: impl Fn(&[u8]) -> [[u8; 4]; 16]) -> Image {
	let (across, _) = blocks(width, height);
	let mut rgba = vec![0u8; (width * height * 4) as usize];
	for (index, block) in blocks_bytes.chunks(BLOCK_BYTES).enumerate() {
		let (bx, by) = (index as u32 % across, index as u32 / across);
		for (i, texel) in decode(block).iter().enumerate() {
			let (x, y) = (bx * 4 + i as u32 % 4, by * 4 + i as u32 / 4);
			if x < width && y < height {
				let at = ((y * width + x) * 4) as usize;
				rgba[at..at + 4].copy_from_slice(texel);
			}
		}
	}
	Image { width, height, rgba }
}

/// BC1 blocks back to RGBA (the reference decoder: alpha 255).
pub fn decode_bc1(width: u32, height: u32, blocks_bytes: &[u8]) -> Image {
	decode_image(width, height, blocks_bytes, |b| bc1::decode(b.try_into().expect("a block")))
}

/// BC4 blocks back to RGBA, the channel in red, green and blue (alpha 255).
pub fn decode_bc4(width: u32, height: u32, blocks_bytes: &[u8]) -> Image {
	decode_image(width, height, blocks_bytes, |b| {
		bc4::decode(b.try_into().expect("a block")).map(|v| [v, v, v, 255])
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A smooth, low-contrast image like the water's: gradients and soft noise.
	pub(crate) fn smooth(width: u32, height: u32) -> Image {
		let mut rgba = Vec::with_capacity((width * height * 4) as usize);
		for y in 0..height {
			for x in 0..width {
				let n = ((x * 7919 + y * 104_729) % 13) as f32 - 6.0;
				let f = |base: f32, k: f32| (base + k * ((x as f32 / 9.0).sin() + (y as f32 / 13.0).cos()) + n * 0.4).round() as u8;
				rgba.extend_from_slice(&[f(26.0, 5.0), f(48.0, 9.0), f(46.0, 8.0), 255]);
			}
		}
		Image { width, height, rgba }
	}

	#[test]
	fn threads_give_the_same_bytes() {
		let image = smooth(64, 40);
		assert_eq!(encode_bc1(&Plane::rgba(&image), 1), encode_bc1(&Plane::rgba(&image), 5));
		assert_eq!(encode_bc4(&Plane::rgba(&image), 1), encode_bc4(&Plane::rgba(&image), 3));
		assert_eq!(encode_bc1(&Plane::rgba(&image), 1).len(), 16 * 10 * BLOCK_BYTES);
	}

	#[test]
	fn small_levels_repeat_their_edges() {
		// A 2×1 mip level: one block, its texels past the edge the edge's.
		let image = Image { width: 2, height: 1, rgba: vec![10, 20, 30, 255, 200, 100, 50, 255] };
		let back = decode_bc1(2, 1, &encode_bc1(&Plane::rgba(&image), 1));
		assert_eq!((back.width, back.height), (2, 1));
		for (got, want) in back.rgba.chunks(4).zip(image.rgba.chunks(4)) {
			for c in 0..3 {
				assert!(got[c].abs_diff(want[c]) <= 4, "{got:?} for {want:?}");
			}
		}
		let grey = decode_bc4(2, 1, &encode_bc4(&Plane::rgba(&image), 1));
		assert_eq!([grey.rgba[0], grey.rgba[4]], [10, 200], "two values are the endpoints");
	}
}
