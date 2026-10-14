//! An atlas's frames, for keeping only the drawn ones on the GPU (texture
//! residency). A frame is a brush frame's rectangle of texels in its atlas;
//! the renderer keeps it in a page of its own making, cut out with a region around it.
//!
//! The region is what makes a frame drawn from a page look exactly as from its atlas:
//! - it adds [`GUTTER`] texels on each side, so filtering at the frame's edge reads the
//!   same neighbours (the atlas's next sprite or its empty space) as in the atlas;
//! - its edges are on [`ALIGN`]-texel boundaries of the atlas, and a page places it on
//!   such a boundary too, so mip levels 1–3 (2×2, 4×4 and 8×8 blocks: the depth of
//!   field's level 2, a zoom-out's) average the same texels as the atlas's.

use std::collections::HashMap;

use crate::{Image, SpriteRegistry};

/// Texels kept around a frame on each side.
pub const GUTTER: u32 = 8;

/// The boundary a region's edges are on, in the atlas and in a page.
pub const ALIGN: u32 = 8;

/// A rectangle of texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TexelRect {
	pub x: u32,
	pub y: u32,
	pub w: u32,
	pub h: u32,
}

impl TexelRect {
	/// The texels `uv` (left, right, top, bottom in `0..=1`, either way round) covers in
	/// an atlas of `width` × `height`.
	pub fn of_uv([u0, u1, v0, v1]: [f32; 4], width: u32, height: u32) -> TexelRect {
		let at = |t: f32, size: u32| (t * size as f32).round().clamp(0.0, size as f32) as u32;
		let (x0, x1) = (at(u0.min(u1), width), at(u0.max(u1), width));
		let (y0, y1) = (at(v0.min(v1), height), at(v0.max(v1), height));
		TexelRect {
			x: x0,
			y: y0,
			w: x1 - x0,
			h: y1 - y0,
		}
	}

	/// Whether `other` lies within it.
	pub fn contains(&self, other: &TexelRect) -> bool {
		other.x >= self.x && other.y >= self.y && other.x + other.w <= self.x + self.w && other.y + other.h <= self.y + self.h
	}

	/// The region cut out around it in an atlas of `width` × `height`: [`GUTTER`] more on
	/// each side, its edges on [`ALIGN`] boundaries, within the atlas.
	pub fn region(&self, width: u32, height: u32) -> TexelRect {
		let down = |v: u32| v.saturating_sub(GUTTER) / ALIGN * ALIGN;
		let up = |v: u32, size: u32| (v + GUTTER).div_ceil(ALIGN).saturating_mul(ALIGN).min(size);
		let (x0, y0) = (down(self.x), down(self.y));
		let (x1, y1) = (up(self.x + self.w, width), up(self.y + self.h, height));
		TexelRect {
			x: x0,
			y: y0,
			w: x1.saturating_sub(x0),
			h: y1.saturating_sub(y0),
		}
	}
}

/// Every atlas's frames in `registry`, by the atlas's path: each rectangle once, in
/// order.
pub fn atlas_frames(registry: &SpriteRegistry) -> HashMap<String, Vec<TexelRect>> {
	let mut atlases: HashMap<String, Vec<TexelRect>> = HashMap::new();
	for loaded in registry.sheets() {
		let sheet = &loaded.sheet;
		let (width, height) = (u32::from(sheet.header.width), u32::from(sheet.header.height));
		let frames = atlases.entry(sheet.texture.clone()).or_default();
		for sprite in &sheet.sprites {
			for frame in &sprite.frames {
				let rect = TexelRect::of_uv(frame.uv, width, height);
				if rect.w > 0 && rect.h > 0 {
					frames.push(rect);
				}
			}
		}
	}
	for frames in atlases.values_mut() {
		frames.sort();
		frames.dedup();
	}
	atlases
}

/// `region` of `atlas`, as an image of its own.
pub fn cut(atlas: &Image, region: TexelRect) -> Image {
	let mut rgba = Vec::with_capacity((region.w * region.h * 4) as usize);
	for y in region.y..region.y + region.h {
		let start = ((y * atlas.width + region.x) * 4) as usize;
		rgba.extend_from_slice(&atlas.rgba[start..start + (region.w * 4) as usize]);
	}
	Image {
		width: region.w,
		height: region.h,
		rgba,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_frames_texels_from_its_uv() {
		let rect = TexelRect::of_uv([0.25, 0.5, 0.75, 0.5], 64, 32);
		assert_eq!(rect, TexelRect { x: 16, y: 16, w: 16, h: 8 }, "either way round");
		assert!(rect.contains(&TexelRect { x: 20, y: 18, w: 4, h: 4 }));
		assert!(!rect.contains(&TexelRect { x: 30, y: 18, w: 4, h: 4 }));
	}

	#[test]
	fn the_region_adds_a_gutter_on_aligned_edges_within_the_atlas() {
		let frame = TexelRect { x: 21, y: 3, w: 10, h: 5 };
		let region = frame.region(64, 32);
		assert_eq!(region, TexelRect { x: 8, y: 0, w: 32, h: 16 });
		assert!(region.contains(&frame));
		assert!(region.x % ALIGN == 0 && region.y % ALIGN == 0);
		assert!(frame.x - region.x >= GUTTER && region.x + region.w - (frame.x + frame.w) >= GUTTER);
		let corner = TexelRect { x: 60, y: 30, w: 4, h: 2 };
		assert_eq!(corner.region(64, 32), TexelRect { x: 48, y: 16, w: 16, h: 16 }, "clamped to the atlas");
	}

	#[test]
	fn a_cut_is_the_regions_texels() {
		let atlas = Image {
			width: 4,
			height: 3,
			rgba: (0..48).collect(),
		};
		let image = cut(&atlas, TexelRect { x: 1, y: 1, w: 2, h: 2 });
		assert_eq!((image.width, image.height), (2, 2));
		assert_eq!(image.rgba, [20, 21, 22, 23, 24, 25, 26, 27, 36, 37, 38, 39, 40, 41, 42, 43]);
	}
}
