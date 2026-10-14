//! Which parts of each atlas the frames draw, for the texture residency design:
//! per atlas, the area its quads sampled (as a grid of [`CELL`]-texel squares, so
//! the same frame drawn again counts once), and whether a draw sampled its mips (a
//! quad drawn smaller than its texels, or below the battle camera's level with the depth
//! of field on). Off unless asked for ([`crate::WorldRenderer::track_usage`]).

use std::collections::HashMap;

use crate::sprite::Quad;
use crate::world::WorldQuads;

/// The side of a coverage cell, in texels.
const CELL: u32 = 8;

/// One atlas's use so far.
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasUse {
	pub path: String,
	pub width: u32,
	pub height: u32,
	/// Texels in the cells any quad sampled.
	pub drawn_texels: u64,
	/// Whether a draw sampled its mips.
	pub mips: bool,
}

/// The atlases' use, gathered over draws.
#[derive(Default)]
pub struct TextureUsage {
	atlases: HashMap<String, Coverage>,
}

struct Coverage {
	width: u32,
	height: u32,
	cells: Vec<bool>,
	mips: bool,
}

impl Coverage {
	fn new(width: u32, height: u32) -> Coverage {
		let (cw, ch) = (width.div_ceil(CELL), height.div_ceil(CELL));
		Coverage {
			width,
			height,
			cells: vec![false; (cw * ch) as usize],
			mips: false,
		}
	}

	/// Mark the texels `uv` spans (normalized), clamped to the atlas.
	fn mark(&mut self, uv: &[[f32; 2]; 4]) {
		let (u0, u1) = span(uv.iter().map(|c| c[0]));
		let (v0, v1) = span(uv.iter().map(|c| c[1]));
		// The first texel and the last (the span's end is exclusive).
		let first = |t: f32, size: u32| ((t * size as f32).floor().max(0.0) as u32).min(size - 1);
		let last = |t: f32, size: u32| (((t * size as f32).ceil() as u32).max(1) - 1).min(size - 1);
		let (x0, x1) = (first(u0, self.width) / CELL, last(u1, self.width) / CELL);
		let (y0, y1) = (first(v0, self.height) / CELL, last(v1, self.height) / CELL);
		let row = self.width.div_ceil(CELL);
		for y in y0..=y1 {
			for x in x0..=x1 {
				self.cells[(y * row + x) as usize] = true;
			}
		}
	}

	fn drawn_texels(&self) -> u64 {
		let row = self.width.div_ceil(CELL);
		self.cells
			.iter()
			.enumerate()
			.filter(|(_, on)| **on)
			.map(|(i, _)| {
				let (x, y) = (i as u32 % row, i as u32 / row);
				let w = CELL.min(self.width - x * CELL);
				let h = CELL.min(self.height - y * CELL);
				u64::from(w) * u64::from(h)
			})
			.sum()
	}
}

/// The least and the greatest of `values`.
fn span(values: impl Iterator<Item = f32>) -> (f32, f32) {
	values.fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
}

impl TextureUsage {
	/// Record `map`'s quads, drawn at `zoom` screen pixels per world unit, with the
	/// depth of field blurring what's below `dof_below` (the camera's level), if on.
	/// `size_of` gives a loaded atlas's size (others aren't recorded).
	pub(crate) fn record(
		&mut self,
		map: &WorldQuads,
		zoom: f32,
		dof_below: Option<u32>,
		size_of: impl Fn(&str) -> Option<(u32, u32)>,
	) {
		for quad in &map.quads {
			let path = &map.atlases.paths[quad.texture];
			let Some((width, height)) = size_of(path) else { continue };
			let coverage = self.atlases.entry(path.clone()).or_insert_with(|| Coverage::new(width, height));
			coverage.mark(&quad.uv);
			coverage.mips |= minified(quad, zoom, width) || dof_below.is_some_and(|level| quad.level < level);
		}
	}

	/// Every atlas drawn so far, the most drawn first.
	pub fn report(&self) -> Vec<AtlasUse> {
		let mut atlases: Vec<AtlasUse> = self
			.atlases
			.iter()
			.map(|(path, c)| AtlasUse {
				path: path.clone(),
				width: c.width,
				height: c.height,
				drawn_texels: c.drawn_texels(),
				mips: c.mips,
			})
			.collect();
		atlases.sort_by(|a, b| b.drawn_texels.cmp(&a.drawn_texels).then_with(|| a.path.cmp(&b.path)));
		atlases
	}
}

/// Whether `quad` draws fewer screen pixels across than the texels it samples (so the
/// GPU reads its mips).
fn minified(quad: &Quad, zoom: f32, atlas_width: u32) -> bool {
	let (x0, x1) = span(quad.corners.iter().map(|c| c[0]));
	let (u0, u1) = span(quad.uv.iter().map(|c| c[0]));
	let screen = (x1 - x0) * zoom;
	let texels = (u1 - u0) * atlas_width as f32;
	texels > 0.0 && screen < texels * 0.99
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_frame_drawn_twice_counts_once() {
		let mut c = Coverage::new(64, 32);
		let uv = [[0.0, 0.0], [0.25, 0.0], [0.25, 0.5], [0.0, 0.5]];
		c.mark(&uv);
		c.mark(&uv);
		assert_eq!(c.drawn_texels(), 16 * 16, "a quarter of the width, half the height");
		c.mark(&[[0.5, 0.5], [1.0, 0.5], [1.0, 1.0], [0.5, 1.0]]);
		assert_eq!(c.drawn_texels(), 16 * 16 + 32 * 16);
	}
}
