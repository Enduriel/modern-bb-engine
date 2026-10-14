//! An atlas's frames, for keeping only the drawn ones on the GPU (texture
//! residency). A frame is a brush frame's rectangle of texels in its atlas;
//! the renderer keeps it in a page of its own making, cut out with a region around it.
//!
//! The region ([`Region`]) is what makes a frame drawn from a page look exactly as from
//! its atlas:
//! - it adds [`GUTTER`] texels on each side, so filtering at the frame's edge reads the
//!   same neighbours (the atlas's next sprite or its empty space) as in the atlas; past
//!   the atlas's own edge it repeats the edge texels, as the atlas's sampler clamps;
//! - its edges are on [`ALIGN`]-texel boundaries of the atlas, and a page places it on
//!   such a boundary too, so its first [`LEVELS`] mip levels line up with the atlas's:
//!   they're cut from the atlas's own mip chain ([`cut_levels`], clamped at each level's
//!   edge as the atlas's sampler clamps), and a frame drawn from a page samples the
//!   same texels at every level the game reads (the depth of field's level 2, a
//!   zoom-out's 1 and 2).

use std::collections::HashMap;

use crate::{Image, SpriteRegistry};

/// Texels kept around a frame on each side.
pub const GUTTER: u32 = 8;

/// The boundary a region's edges are on, in the atlas and in a page.
pub const ALIGN: u32 = 8;

/// The mip levels a frame's region keeps, level 0 its own texels: as many as [`ALIGN`]
/// allows to line up (8 = 2³, so levels 0–3).
pub const LEVELS: usize = 4;

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

	/// The region cut out around it: [`GUTTER`] more on each side, its edges on [`ALIGN`]
	/// boundaries (past the atlas's edge, if it's there).
	pub fn region(&self) -> Region {
		let (gutter, align) = (GUTTER as i64, ALIGN as i64);
		let down = |v: u32| (i64::from(v) - gutter).div_euclid(align) * align;
		let up = |v: u32| (i64::from(v) + gutter + align - 1).div_euclid(align) * align;
		let (x0, y0) = (down(self.x), down(self.y));
		let (x1, y1) = (up(self.x + self.w), up(self.y + self.h));
		Region {
			x: x0 as i32,
			y: y0 as i32,
			w: (x1 - x0) as u32,
			h: (y1 - y0) as u32,
		}
	}
}

/// The texels cut out around a frame, in its atlas's coordinates (which it may reach
/// past, see [`cut`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
	pub x: i32,
	pub y: i32,
	pub w: u32,
	pub h: u32,
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

/// The frames of each animated sprite (a brush with more than one frame) in
/// `registry`, by atlas: each of its frames to all of them, in order (a frame several
/// animations share goes with the first). An animation's frames load together, so it
/// never waits on its next frame.
pub fn animations(registry: &SpriteRegistry) -> HashMap<String, HashMap<TexelRect, std::sync::Arc<[TexelRect]>>> {
	let mut atlases: HashMap<String, HashMap<TexelRect, std::sync::Arc<[TexelRect]>>> = HashMap::new();
	for loaded in registry.sheets() {
		let sheet = &loaded.sheet;
		let (width, height) = (u32::from(sheet.header.width), u32::from(sheet.header.height));
		for sprite in sheet.sprites.iter().filter(|s| s.frames.len() > 1) {
			let mut frames: Vec<TexelRect> = sprite
				.frames
				.iter()
				.map(|f| TexelRect::of_uv(f.uv, width, height))
				.filter(|r| r.w > 0 && r.h > 0)
				.collect();
			frames.dedup();
			if frames.len() < 2 {
				continue;
			}
			let group: std::sync::Arc<[TexelRect]> = frames.into();
			let groups = atlases.entry(sheet.texture.clone()).or_default();
			for frame in group.iter() {
				groups.entry(*frame).or_insert_with(|| group.clone());
			}
		}
	}
	atlases
}

/// `region` and its first [`LEVELS`] mip levels, cut from the atlas's mip `chain` (level
/// 0 first; past a level's edge, its edge texels repeated).
pub fn cut_levels(chain: &[Image], region: Region) -> Vec<Image> {
	(0..LEVELS)
		.map(|level| {
			let at = &chain[level.min(chain.len() - 1)];
			let shift = level.min(chain.len() - 1) as u32;
			cut(
				at,
				Region {
					x: region.x >> shift,
					y: region.y >> shift,
					w: (region.w >> level).max(1),
					h: (region.h >> level).max(1),
				},
			)
		})
		.collect()
}

/// `region` of `atlas`, as an image of its own; past the atlas's edge, its edge texels
/// repeated.
pub fn cut(atlas: &Image, region: Region) -> Image {
	let mut rgba = Vec::with_capacity((region.w * region.h * 4) as usize);
	let clamp = |v: i64, size: u32| v.clamp(0, i64::from(size) - 1) as u32;
	for y in 0..i64::from(region.h) {
		let sy = clamp(i64::from(region.y) + y, atlas.height);
		for x in 0..i64::from(region.w) {
			let sx = clamp(i64::from(region.x) + x, atlas.width);
			let at = ((sy * atlas.width + sx) * 4) as usize;
			rgba.extend_from_slice(&atlas.rgba[at..at + 4]);
		}
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
	fn the_region_adds_a_gutter_on_aligned_edges() {
		let frame = TexelRect { x: 21, y: 3, w: 10, h: 5 };
		let region = frame.region();
		assert_eq!(region, Region { x: 8, y: -8, w: 32, h: 24 }, "past the atlas's top");
		assert!(region.x % ALIGN as i32 == 0 && region.y % ALIGN as i32 == 0);
		assert!(frame.x as i32 - region.x >= GUTTER as i32);
		assert!(region.x + region.w as i32 - (frame.x + frame.w) as i32 >= GUTTER as i32);
	}

	#[test]
	fn a_regions_levels_come_from_the_atlass_chain() {
		let atlas = Image {
			width: 16,
			height: 16,
			rgba: (0..16 * 16 * 4).map(|i| (i * 13 % 256) as u8).collect(),
		};
		let chain = crate::mips::mip_chain(&atlas);
		let region = Region { x: -8, y: 8, w: 16, h: 8 };
		let levels = cut_levels(&chain, region);
		assert_eq!(levels.iter().map(|l| (l.width, l.height)).collect::<Vec<_>>(), [(16, 8), (8, 4), (4, 2), (2, 1)]);
		assert_eq!(levels[1], cut(&chain[1], Region { x: -4, y: 4, w: 8, h: 4 }), "level 1 of the region is the region of level 1");
	}

	#[test]
	fn past_the_atlas_its_edge_texels_repeat() {
		let atlas = Image {
			width: 2,
			height: 1,
			rgba: vec![1, 1, 1, 1, 2, 2, 2, 2],
		};
		let image = cut(&atlas, Region { x: -1, y: 0, w: 4, h: 2 });
		let firsts: Vec<u8> = image.rgba.chunks(4).map(|t| t[0]).collect();
		assert_eq!(firsts, [1, 1, 2, 2, 1, 1, 2, 2], "clamped, as the atlas's sampler");
	}

	#[test]
	fn a_cut_is_the_regions_texels() {
		let atlas = Image {
			width: 4,
			height: 3,
			rgba: (0..48).collect(),
		};
		let image = cut(&atlas, Region { x: 1, y: 1, w: 2, h: 2 });
		assert_eq!((image.width, image.height), (2, 2));
		assert_eq!(image.rgba, [20, 21, 22, 23, 24, 25, 26, 27, 36, 37, 38, 39, 40, 41, 42, 43]);
	}
}
