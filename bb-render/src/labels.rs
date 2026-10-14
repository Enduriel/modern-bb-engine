//! World entity labels (a settlement's or party's name): text in the engine's bitmap
//! font on a translucent plate.
//!
//! Measured against screenshots of the original (Lydersted at zoom 1 and 2), not yet
//! read from its code:
//! - the font is `cinzel_bold_20`, in the label's colour, unaffected by ambient light;
//! - the text pen starts at the entity's x (plus the label's offset), and the glyph
//!   cells' tops are at the entity's y (the text matches at correlation 0.99);
//! - the plate is black at alpha 100, from 2 px left of the pen to 4 px past the
//!   text's advance, and the cell height plus 2 px tall, its top at the entity's y;
//! - labels keep their screen size at any zoom, and draw above everything else.

use bb_assets::BitmapFont;
use squirrel_bb::scene::SceneLabel;

use crate::sprite::Quad;

/// The font labels are drawn in, and its atlas's path.
#[derive(Clone, Copy)]
pub struct LabelFont<'a> {
	pub font: &'a BitmapFont,
	pub texture: &'a str,
}

/// The texture key of a solid fill: a 1×1 white texture tinted by the quad's colour.
pub const WHITE: &str = "@white";

/// The plate's colour.
const PLATE: [u8; 4] = [0, 0, 0, 100];

/// The quads of `label` at `anchor` (world units, y up), with `pixel` world units per
/// screen pixel, each with the texture key it samples.
pub(crate) fn label_quads<'a>(
	label: &SceneLabel,
	anchor: [f32; 2],
	pixel: f32,
	font: LabelFont<'a>,
) -> Vec<(&'a str, Quad)> {
	let text = label.text.as_slice();
	let [x, y] = [
		anchor[0] + label.offset.0 as f32 * pixel,
		anchor[1] + label.offset.1 as f32 * pixel,
	];
	let advance = font.font.text_width(text) as f32;
	let (cell_w, cell_h) = (font.font.cell_width as f32, font.font.cell_height as f32);
	let mut quads = vec![(
		WHITE,
		solid(
			[x - 2.0 * pixel, y],
			[x + (advance + 4.0) * pixel, y - (cell_h + 2.0) * pixel],
			PLATE,
		),
	)];
	let [a, r, g, b] = label.color.to_be_bytes();
	let color = [r, g, b, a];
	let mut pen = x;
	let (top, bottom) = (y, y - cell_h * pixel);
	for &code in text {
		if let Some([u0, u1, v0, v1]) = font.font.glyph_uv(code) {
			let right = pen + cell_w * pixel;
			quads.push((
				font.texture,
				Quad {
					texture: 0,
					corners: [[pen, top], [right, top], [right, bottom], [pen, bottom]],
					uv: [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
					color,
					overlay: [0; 4],
					factor: 0.0,
					saturation: 1.0,
					flags: 1,
				},
			));
		}
		pen += f32::from(font.font.advances[usize::from(code)]) * pixel;
	}
	quads
}

/// A rectangle filled with `color`, from its top-left to its bottom-right corner.
fn solid(top_left: [f32; 2], bottom_right: [f32; 2], color: [u8; 4]) -> Quad {
	let ([x0, y0], [x1, y1]) = (top_left, bottom_right);
	Quad {
		texture: 0,
		corners: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
		uv: [[0.5, 0.5]; 4],
		color,
		overlay: [0; 4],
		factor: 0.0,
		saturation: 1.0,
		flags: 1,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

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

	fn label(text: &str) -> SceneLabel {
		SceneLabel {
			name: "name".into(),
			text: text.into(),
			visible: true,
			color: 0x80ff_0000,
			offset: (0.0, 0.0),
		}
	}

	#[test]
	fn the_plate_wraps_the_text_from_the_anchor() {
		let font = font();
		let quads = label_quads(
			&label("AB"),
			[100.0, 50.0],
			1.0,
			LabelFont {
				font: &font,
				texture: "gfx/f.png",
			},
		);
		let (key, plate) = &quads[0];
		assert_eq!(*key, WHITE);
		// 2 px before the pen to 4 px past the 22 px advance; 22 px tall, top at y.
		assert_eq!(
			plate.corners,
			[[98.0, 50.0], [126.0, 50.0], [126.0, 28.0], [98.0, 28.0]]
		);
		assert_eq!(plate.color, [0, 0, 0, 100]);
		// Glyphs: cells at the pen, their tops at the anchor, advancing by width.
		assert_eq!(quads.len(), 3);
		assert_eq!(quads[1].1.corners[0], [100.0, 50.0]);
		assert_eq!(quads[2].1.corners[0], [110.0, 50.0]);
		assert_eq!(quads[1].1.color, [255, 0, 0, 128], "ARGB becomes RGBA");
	}

	#[test]
	fn labels_keep_their_screen_size() {
		let font = font();
		let quads = label_quads(
			&label("A"),
			[0.0, 0.0],
			2.0,
			LabelFont {
				font: &font,
				texture: "gfx/f.png",
			},
		);
		assert_eq!(quads[0].1.corners[2], [(10.0 + 4.0) * 2.0, -22.0 * 2.0]);
	}
}
