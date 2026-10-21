//! A world map quad as the kept map holds it on the CPU (see `crate::kept_map`), for
//! locating it again in the pages (a page evicted or packed again) and for telling a
//! changed detail apart.
//!
//! The map's terrain and details are tens of thousands of quads that all leave a
//! [`Quad`]'s per-corner colours, overlay, flash factor and depth level at their
//! defaults (see `crate::world::frame_quad`). Only the rest is kept: two thirds of a
//! [`Quad`]'s bytes, with no `Option` tag for an empty slot.

use crate::sprite::Quad;

/// A map quad (or none: a hidden detail, an unknown brush), in its atlas's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MapQuad {
	corners: [[f32; 2]; 4],
	uv: [[f32; 2]; 4],
	/// Its atlas, an index into the kept map's list; [`NONE`] for no quad.
	texture: u32,
	color: [u8; 4],
	saturation: f32,
	flags: u32,
}

/// The `texture` of no quad.
const NONE: u32 = u32::MAX;

impl MapQuad {
	/// No quad.
	pub(crate) const EMPTY: MapQuad = MapQuad {
		corners: [[0.0; 2]; 4],
		uv: [[0.0; 2]; 4],
		texture: NONE,
		color: [0; 4],
		saturation: 0.0,
		flags: 0,
	};

	/// `quad` kept: its corner colours, overlay, factor and level must be the defaults
	/// a map quad has.
	pub(crate) fn of(quad: Option<Quad>) -> MapQuad {
		let Some(quad) = quad else { return MapQuad::EMPTY };
		debug_assert!(
			quad.corner_colors.is_none() && quad.overlay == [0; 4] && quad.factor == 0.0 && quad.level == 0,
			"a map quad keeps no corner colours, overlay or level"
		);
		debug_assert!(quad.texture < NONE as usize, "an atlas index");
		MapQuad {
			corners: quad.corners,
			uv: quad.uv,
			texture: quad.texture as u32,
			color: quad.color,
			saturation: quad.saturation,
			flags: quad.flags,
		}
	}

	/// The quad, if there is one.
	pub(crate) fn get(&self) -> Option<Quad> {
		(self.texture != NONE).then_some(Quad {
			texture: self.texture as usize,
			corners: self.corners,
			uv: self.uv,
			color: self.color,
			corner_colors: None,
			overlay: [0; 4],
			factor: 0.0,
			saturation: self.saturation,
			flags: self.flags,
			level: 0,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_map_quad_comes_back_as_it_went_in_in_two_thirds_of_the_bytes() {
		let quad = Quad {
			texture: 3,
			corners: [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0], [7.0, 8.0]],
			uv: [[0.1, 0.2], [0.3, 0.4], [0.5, 0.6], [0.7, 0.8]],
			color: [1, 2, 3, 4],
			corner_colors: None,
			overlay: [0; 4],
			factor: 0.0,
			saturation: 0.5,
			flags: 1,
			level: 0,
		};
		assert_eq!(MapQuad::of(Some(quad)).get(), Some(quad));
		assert_eq!(MapQuad::of(None).get(), None);
		assert_eq!(MapQuad::of(None), MapQuad::EMPTY);
		assert_eq!(std::mem::size_of::<MapQuad>(), 80);
		assert!(std::mem::size_of::<Option<Quad>>() >= 120);
	}
}
