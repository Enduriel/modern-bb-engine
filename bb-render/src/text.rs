//! Text nodes: a line of bitmap-font glyphs, laid out as the original's text node
//! does (layout `0x5611a0`, quads `0x561350`).
//!
//! - A line is as wide as its characters' advances and one cell tall. Alignment moves
//!   the text's top-left from the node's position by a fraction of that size (region
//!   names centre both ways).
//! - The node draws at `s` world units per font pixel: `max(1, Zoom × scale)`, where
//!   `Zoom` is the camera's world units per screen pixel. So text keeps its full
//!   pixel size on screen when zoomed in and never shrinks below `scale` times it
//!   when zoomed out. A node with flag `0x40` and a scale of exactly 1 always draws at
//!   `s = 1`.
//! - The pen starts at the position plus the alignment offset times `s`, rounded half
//!   away from zero. Glyph tops sit on the pen's y, and each glyph is one advance wide
//!   (the left part of its cell, where the glyph is) and one cell tall.
//! - A rotated node turns counter-clockwise about its position plus the unscaled
//!   alignment offset, rounded.
//! - Its quads take vertex flag 0 (`0x561d2b`): lit by the scene's ambient colour like
//!   any sprite, warm at dawn and blue at night.

use crate::labels::LabelFont;
use crate::sprite::Quad;

/// One line of text to draw.
pub(crate) struct TextRun<'a> {
	pub text: &'a [u8],
	pub font: LabelFont<'a>,
	/// The node's position, world units.
	pub pos: [f32; 2],
	/// From the position to the text's top-left, in font pixels, y up.
	pub offset: [f32; 2],
	/// World units per font pixel (see [`text_scale`]).
	pub scale: f32,
	/// Degrees, counter-clockwise.
	pub rotation: f32,
	pub color: [u8; 4],
}

/// World units per font pixel for a node of `scale` seen at `zoom` world units per
/// screen pixel; `fixed` is the node's flag `0x40`.
pub(crate) fn text_scale(zoom: f32, scale: f32, fixed: bool) -> f32 {
	if fixed && scale == 1.0 {
		1.0
	} else {
		(zoom * scale).max(1.0)
	}
}

/// C's `(int)(x ± 0.5)`: rounding half away from zero.
fn round(x: f32) -> f32 {
	(if x < 0.0 { x - 0.5 } else { x + 0.5 }).trunc()
}

/// The glyph quads of `run`, each with the atlas it samples.
pub(crate) fn glyph_quads<'a>(run: &TextRun<'a>) -> Vec<(&'a str, Quad)> {
	let font = run.font.font;
	let s = run.scale;
	let pen_x = round(run.pos[0] + run.offset[0] * s);
	let top = round(run.pos[1] + run.offset[1] * s);
	let bottom = top - font.cell_height as f32 * s;
	let pivot = [
		round(run.pos[0] + run.offset[0]),
		round(run.pos[1] + run.offset[1]),
	];
	let (sin, cos) = run.rotation.to_radians().sin_cos();
	let turn = |[x, y]: [f32; 2]| {
		if run.rotation == 0.0 {
			return [x, y];
		}
		let (dx, dy) = (x - pivot[0], y - pivot[1]);
		[
			pivot[0] + dx * cos - dy * sin,
			pivot[1] + dx * sin + dy * cos,
		]
	};
	let mut quads = Vec::new();
	let mut pen = 0.0;
	for &code in run.text {
		let advance = f32::from(font.advances[usize::from(code)]);
		if let Some([u0, _, v0, v1]) = font.glyph_uv(code) {
			let (x0, x1) = (pen_x + pen, pen_x + pen + advance * s);
			let u1 = u0 + advance / font.atlas_width as f32;
			quads.push((
				run.font.texture,
				Quad {
					texture: 0,
					corners: [
						turn([x0, top]),
						turn([x1, top]),
						turn([x1, bottom]),
						turn([x0, bottom]),
					],
					uv: [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
					color: run.color,
					corner_colors: None,
					overlay: [0; 4],
					factor: 0.0,
					saturation: 1.0,
					flags: 0,
					level: 0,
				},
			));
		}
		pen += advance * s;
	}
	quads
}

#[cfg(test)]
mod tests {
	use super::*;
	use bb_assets::BitmapFont;

	fn font() -> BitmapFont {
		let mut advances = [0u8; 256];
		advances[b'A' as usize] = 10;
		advances[b'B' as usize] = 12;
		BitmapFont {
			atlas_width: 512,
			atlas_height: 256,
			cell_width: 22,
			cell_height: 20,
			first: 32,
			advances,
		}
	}

	fn run<'a>(font: &'a BitmapFont, text: &'a [u8]) -> TextRun<'a> {
		TextRun {
			text,
			font: LabelFont {
				font,
				texture: "gfx/f.png",
			},
			pos: [100.0, 50.0],
			offset: [0.0, 0.0],
			scale: 1.0,
			rotation: 0.0,
			color: [255; 4],
		}
	}

	#[test]
	fn the_scale_keeps_pixel_size_when_zoomed_out() {
		assert_eq!(text_scale(0.5, 1.0, false), 1.0, "zoomed in: full size");
		assert_eq!(
			text_scale(2.0, 1.0, false),
			2.0,
			"zoomed out: same screen size"
		);
		assert_eq!(
			text_scale(2.0, 0.4, true),
			1.0,
			"shrinks until zoom × scale is 1"
		);
		assert_eq!(text_scale(4.0, 0.4, true), 1.6);
		assert_eq!(text_scale(4.0, 1.0, true), 1.0, "fixed at scale 1");
	}

	#[test]
	fn glyphs_are_one_advance_wide_from_a_rounded_pen() {
		let font = font();
		let mut r = run(&font, b"AB");
		r.pos = [100.4, 50.6];
		r.offset = [-11.0, 10.0];
		r.scale = 2.0;
		let quads = glyph_quads(&r);
		assert_eq!(quads.len(), 2);
		// Pen at round(100.4 - 22) = 78, top at round(50.6 + 20) = 71, 40 units tall.
		assert_eq!(
			quads[0].1.corners,
			[[78.0, 71.0], [98.0, 71.0], [98.0, 31.0], [78.0, 31.0]]
		);
		assert_eq!(quads[1].1.corners[0], [98.0, 71.0]);
		// The glyph's part of its cell: 10 of 512 texels across.
		let [u0, _, _, _] = font.glyph_uv(b'A').unwrap();
		assert_eq!(quads[0].1.uv[1][0], u0 + 10.0 / 512.0);
	}

	#[test]
	fn rotation_turns_counter_clockwise_about_the_unscaled_corner() {
		let font = font();
		let mut r = run(&font, b"A");
		r.offset = [-5.0, 10.0];
		r.rotation = 90.0;
		let quads = glyph_quads(&r);
		// Pivot (95, 60); the top-right corner (105, 60) turns to (95, 70).
		let [x, y] = quads[0].1.corners[1];
		assert!(
			(x - 95.0).abs() < 1e-4 && (y - 70.0).abs() < 1e-4,
			"{x}, {y}"
		);
	}
}
