//! World entity labels (a settlement's or party's name): text in the engine's bitmap
//! font on a translucent plate.
//!
//! Measured against screenshots of the original (Lydersted at zoom 1 and 2), not yet
//! read from its code:
//! - the font is `cinzel_bold_20`, in the label's colour, lit by the scene's ambient
//!   colour as any sprite is: a label is a text node (`addLabel` makes one, `0x4e738f`),
//!   whose quads take vertex flag 0 (`0x561350`; the first screenshots were of the
//!   paused start, where the ambient is white; frame parity found the tint);
//! - the text pen starts at the entity's x (plus the label's offset), and the glyph
//!   cells' tops are at the entity's y (the text matches at correlation 0.99);
//! - the plate is black at alpha 100, from 2 px left of the pen to 4 px past the
//!   text's advance, and the cell height plus 2 px tall, its top at the entity's y;
//! - labels keep their screen size at any zoom, and draw above everything else.

use std::cell::RefCell;

use bb_assets::BitmapFont;
use squirrel_bb::scene::SceneLabel;

use crate::fonts::decode::{EngineText, engine_text};
use crate::fonts::shaper::{GLYPH_PAGE, GLYPH_PAGES, ShapedText, TextShaper};
use crate::sprite::Quad;

/// The font labels are drawn in, and its atlas's path.
#[derive(Clone, Copy)]
pub struct LabelFont<'a> {
	pub font: &'a BitmapFont,
	pub texture: &'a str,
}

impl LabelFont<'_> {
	/// The bitmap font's name (`gfx/fonts/<name>.png`), for the outline fonts.
	pub fn name(&self) -> &str {
		let file = self.texture.rsplit('/').next().unwrap_or(self.texture);
		file.strip_suffix(".png").unwrap_or(file)
	}
}

/// The texture key of a solid fill: a 1×1 white texture tinted by the quad's colour.
pub const WHITE: &str = "@white";

/// The plate's colour.
const PLATE: [u8; 4] = [0, 0, 0, 100];

/// The quads of `label` at `anchor` (world units, y up), with `pixel` world units per
/// screen pixel, each with the texture key it samples.
///
/// A text the bitmap font can't draw is shaped by `shaper` (see [`crate::fonts`]) and
/// drawn from its glyph pages instead, on the same plate (without a shaper, it isn't
/// drawn).
pub(crate) fn label_quads<'a>(
	label: &SceneLabel,
	anchor: [f32; 2],
	pixel: f32,
	font: LabelFont<'a>,
	shaper: Option<&RefCell<TextShaper>>,
) -> Vec<(&'a str, Quad)> {
	let [x, y] = [
		anchor[0] + label.offset.0 as f32 * pixel,
		anchor[1] + label.offset.1 as f32 * pixel,
	];
	let text = match engine_text(&label.text) {
		EngineText::Bitmap(bytes) => bytes,
		EngineText::Shaped(text) => {
			let Some(shaper) = shaper else { return Vec::new() };
			let shaped = shaper.borrow_mut().shape(font.name(), &text, None).clone();
			return shaped_label_quads(label, [x, y], pixel, font, &shaped);
		}
	};
	let text = text.as_ref();
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
					corner_colors: None,
					overlay: [0; 4],
					factor: 0.0,
					saturation: 1.0,
					flags: 0,
					level: 0,
				},
			));
		}
		pen += f32::from(font.font.advances[usize::from(code)]) * pixel;
	}
	quads
}

/// A shaped label's plate and glyphs: the plate as a bitmap label's, as wide as the
/// shaped text and at least the bitmap font's cell tall; the glyphs from the text's
/// top-left at the anchor.
fn shaped_label_quads<'a>(label: &SceneLabel, [x, y]: [f32; 2], pixel: f32, font: LabelFont<'a>, shaped: &ShapedText) -> Vec<(&'a str, Quad)> {
	let height = shaped.height.max(font.font.cell_height as f32);
	let mut quads = vec![(
		WHITE,
		solid([x - 2.0 * pixel, y], [x + (shaped.width + 4.0) * pixel, y - (height + 2.0) * pixel], PLATE),
	)];
	let [a, r, g, b] = label.color.to_be_bytes();
	for glyph in &shaped.glyphs {
		let [tx, ty, tw, th] = glyph.texels;
		let (left, top) = (x + glyph.left * pixel, y - glyph.top * pixel);
		let (right, bottom) = (left + tw as f32 * pixel, top - th as f32 * pixel);
		let page = GLYPH_PAGE as f32;
		let (u0, u1, v0, v1) = (tx as f32 / page, (tx + tw) as f32 / page, ty as f32 / page, (ty + th) as f32 / page);
		quads.push((
			GLYPH_PAGES[glyph.page],
			Quad {
				texture: 0,
				corners: [[left, top], [right, top], [right, bottom], [left, bottom]],
				uv: [[u0, v0], [u1, v0], [u1, v1], [u0, v1]],
				color: [r, g, b, a],
				corner_colors: None,
				overlay: [0; 4],
				factor: 0.0,
				saturation: 1.0,
				flags: 0,
				level: 0,
			},
		));
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
		corner_colors: None,
		overlay: [0; 4],
		factor: 0.0,
		saturation: 1.0,
		flags: 0,
		level: 0,
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
			text: text.as_bytes().into(),
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
			None,
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
		assert!(quads.iter().all(|(_, q)| q.flags == 0), "lit by the ambient, plate and text");
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
			None,
		);
		assert_eq!(quads[0].1.corners[2], [(10.0 + 4.0) * 2.0, -22.0 * 2.0]);
	}
}
