//! Mip levels on the CPU: an image halved down to 1×1, each texel the rounded mean of
//! the 2×2 block it covers. The GPU makes the same levels for whole atlases
//! (`bb_render`'s `mips`); our sheets keep a frame's first levels cut from these
//! (`crate::frames`).

use crate::Image;

/// `image` and each halving of it down to 1×1. An odd edge drops its last
/// row/column; each texel is the rounded mean of the 2×2 block it covers.
pub fn mip_chain(image: &Image) -> Vec<Image> {
	mip_levels(image, usize::MAX)
}

/// `image` and at most `count - 1` halvings of it, stopping at 1×1.
pub fn mip_levels(image: &Image, count: usize) -> Vec<Image> {
	let mut levels = vec![image.clone()];
	while let Some(last) = levels.last().filter(|i| (i.width > 1 || i.height > 1) && levels.len() < count) {
		levels.push(halve(last));
	}
	levels
}

pub fn halve(src: &Image) -> Image {
	let width = (src.width / 2).max(1);
	let height = (src.height / 2).max(1);
	let mut rgba = Vec::with_capacity((width * height * 4) as usize);
	let texel = |x: u32, y: u32, c: usize| {
		let (x, y) = (x.min(src.width - 1), y.min(src.height - 1));
		u32::from(src.rgba[((y * src.width + x) * 4) as usize + c])
	};
	for y in 0..height {
		for x in 0..width {
			for c in 0..4 {
				let sum = texel(2 * x, 2 * y, c)
					+ texel(2 * x + 1, 2 * y, c)
					+ texel(2 * x, 2 * y + 1, c)
					+ texel(2 * x + 1, 2 * y + 1, c);
				rgba.push(((sum + 2) / 4) as u8);
			}
		}
	}
	Image {
		width,
		height,
		rgba,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn chain_halves_down_to_one_texel() {
		let image = Image {
			width: 8,
			height: 2,
			rgba: vec![0; 8 * 2 * 4],
		};
		let sizes: Vec<(u32, u32)> = mip_chain(&image)
			.iter()
			.map(|i| (i.width, i.height))
			.collect();
		assert_eq!(sizes, [(8, 2), (4, 1), (2, 1), (1, 1)]);
	}

	#[test]
	fn each_texel_averages_its_block_with_rounding() {
		#[rustfmt::skip]
		let image = Image { width: 2, height: 2, rgba: vec![
			0, 10, 255, 1,   1, 10, 255, 1,
			0, 10, 255, 0,   0, 11, 255, 0,
		] };
		assert_eq!(halve(&image).rgba, [0, 10, 255, 1]);
	}
}
