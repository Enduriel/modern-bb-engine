//! Comparing a frame of ours with a screenshot of the original (frame parity): which
//! pixels differ beyond a tolerance, as a heatmap, and the regions they gather in.
//!
//! Text and edges never match exactly (different rasterizers, filtering), so a pixel
//! differs only when a channel is off by more than the tolerance, and a region only
//! counts where enough of a cell's pixels differ. Regions are the differing cells of a
//! coarse grid, joined where they touch.

use bb_assets::Image;

/// The grid's cell size in pixels.
pub const CELL: u32 = 16;

/// How two frames differ.
pub struct FrameDiff {
	/// The pixels that differ beyond the tolerance.
	pub differing: usize,
	pub total: usize,
	/// Ours dimmed to grey, with each differing pixel red by how far off it is.
	pub heat: Image,
	/// Where the differences gather, most differing pixels first.
	pub regions: Vec<Region>,
}

/// A rectangle of touching grid cells that differ.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
	pub x: u32,
	pub y: u32,
	pub width: u32,
	pub height: u32,
	/// Its differing pixels.
	pub pixels: usize,
}

/// Compare `ours` with `original` (the same size). A pixel differs when a colour channel
/// is off by more than `tolerance`; a grid cell when more than `cell_fraction` of its
/// pixels do.
pub fn diff(
	ours: &Image,
	original: &Image,
	tolerance: u8,
	cell_fraction: f32,
) -> Result<FrameDiff, String> {
	if (ours.width, ours.height) != (original.width, original.height) {
		return Err(format!(
			"sizes differ: ours {}x{}, the original's {}x{}",
			ours.width, ours.height, original.width, original.height
		));
	}
	let (width, height) = (ours.width, ours.height);
	let (cols, rows) = (width.div_ceil(CELL), height.div_ceil(CELL));
	let mut cell_counts = vec![0usize; (cols * rows) as usize];
	let mut heat = vec![0u8; ours.rgba.len()];
	let mut differing = 0;
	for y in 0..height {
		for x in 0..width {
			let i = ((y * width + x) * 4) as usize;
			let (a, b) = (&ours.rgba[i..i + 3], &original.rgba[i..i + 3]);
			let off = a
				.iter()
				.zip(b)
				.map(|(p, q)| p.abs_diff(*q))
				.max()
				.unwrap_or(0);
			let grey =
				((u32::from(a[0]) * 3 + u32::from(a[1]) * 6 + u32::from(a[2])) / 10 / 3) as u8;
			let out = &mut heat[i..i + 4];
			if off > tolerance {
				differing += 1;
				cell_counts[((y / CELL) * cols + x / CELL) as usize] += 1;
				out.copy_from_slice(&[128u8.saturating_add(off / 2), grey, grey, 255]);
			} else {
				out.copy_from_slice(&[grey, grey, grey, 255]);
			}
		}
	}
	let cell_pixels = |c: u32, r: u32| {
		let w = CELL.min(width - c * CELL);
		let h = CELL.min(height - r * CELL);
		(w * h) as f32
	};
	let marked: Vec<bool> = (0..rows)
		.flat_map(|r| (0..cols).map(move |c| (c, r)))
		.map(|(c, r)| {
			cell_counts[(r * cols + c) as usize] as f32 > cell_fraction * cell_pixels(c, r)
		})
		.collect();
	let regions = join_cells(&marked, &cell_counts, cols, rows, width, height);
	Ok(FrameDiff {
		differing,
		total: (width * height) as usize,
		heat: Image {
			width,
			height,
			rgba: heat,
		},
		regions,
	})
}

/// The bounding rectangles of the marked cells that touch (8-neighbours), with their
/// differing pixels, most first.
fn join_cells(
	marked: &[bool],
	counts: &[usize],
	cols: u32,
	rows: u32,
	width: u32,
	height: u32,
) -> Vec<Region> {
	let mut seen = vec![false; marked.len()];
	let mut regions = Vec::new();
	for start in 0..marked.len() {
		if !marked[start] || seen[start] {
			continue;
		}
		seen[start] = true;
		let mut stack = vec![start];
		let (mut c0, mut r0, mut c1, mut r1) = (u32::MAX, u32::MAX, 0, 0);
		let mut pixels = 0;
		while let Some(i) = stack.pop() {
			let (c, r) = (i as u32 % cols, i as u32 / cols);
			(c0, r0, c1, r1) = (c0.min(c), r0.min(r), c1.max(c), r1.max(r));
			pixels += counts[i];
			for (dc, dr) in [
				(-1i32, -1i32),
				(0, -1),
				(1, -1),
				(-1, 0),
				(1, 0),
				(-1, 1),
				(0, 1),
				(1, 1),
			] {
				let (nc, nr) = (c as i32 + dc, r as i32 + dr);
				if nc < 0 || nr < 0 || nc >= cols as i32 || nr >= rows as i32 {
					continue;
				}
				let n = (nr as u32 * cols + nc as u32) as usize;
				if marked[n] && !seen[n] {
					seen[n] = true;
					stack.push(n);
				}
			}
		}
		let (x, y) = (c0 * CELL, r0 * CELL);
		regions.push(Region {
			x,
			y,
			width: ((c1 + 1) * CELL).min(width) - x,
			height: ((r1 + 1) * CELL).min(height) - y,
			pixels,
		});
	}
	regions.sort_by(|a, b| b.pixels.cmp(&a.pixels));
	regions
}

#[cfg(test)]
mod tests {
	use super::*;

	fn image(width: u32, height: u32, rgb: [u8; 3]) -> Image {
		let rgba = (0..width * height)
			.flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
			.collect();
		Image {
			width,
			height,
			rgba,
		}
	}

	fn paint(image: &mut Image, x0: u32, y0: u32, w: u32, h: u32, rgb: [u8; 3]) {
		for y in y0..y0 + h {
			for x in x0..x0 + w {
				let i = ((y * image.width + x) * 4) as usize;
				image.rgba[i..i + 3].copy_from_slice(&rgb);
			}
		}
	}

	#[test]
	fn small_differences_are_tolerated() {
		let a = image(64, 64, [100, 100, 100]);
		let b = image(64, 64, [104, 98, 100]);
		let d = diff(&a, &b, 8, 0.05).unwrap();
		assert_eq!((d.differing, d.regions.len()), (0, 0));
	}

	#[test]
	fn separate_blotches_are_separate_regions() {
		let a = image(128, 96, [0, 0, 0]);
		let mut b = a.clone();
		paint(&mut b, 4, 4, 20, 20, [255, 0, 0]);
		paint(&mut b, 90, 60, 10, 10, [0, 255, 0]);
		let d = diff(&a, &b, 8, 0.05).unwrap();
		assert_eq!(d.differing, 500);
		assert_eq!(
			d.regions,
			vec![
				Region {
					x: 0,
					y: 0,
					width: 32,
					height: 32,
					pixels: 400
				},
				Region {
					x: 80,
					y: 48,
					width: 32,
					height: 32,
					pixels: 100
				},
			]
		);
	}

	#[test]
	fn sizes_must_match() {
		assert!(diff(&image(4, 4, [0; 3]), &image(4, 5, [0; 3]), 8, 0.05).is_err());
	}
}
