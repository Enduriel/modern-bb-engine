//! `.fnt` files: the engine's bitmap fonts (`gfx/fonts/*.fnt`, each beside a `.png`
//! atlas of the same name). The engine draws world labels with them; the UI uses
//! TrueType fonts instead.
//!
//! The layout, read from vanilla's three fonts and their atlases:
//!
//! ```text
//! u32 atlas width, u32 atlas height, u32 cell width, u32 cell height,
//! u8 first character (32), [u8; 256] advance width per character code
//! ```
//!
//! Characters from the first one up sit in a grid of cells, `atlas width / cell
//! width` per row, filled left to right and top to bottom of the picture, each glyph
//! at its cell's left edge. Like brush atlases, the picture is stored upside down.
//! Only one byte per character exists, so a font holds at most 256 glyphs.

use crate::reader::Reader;

/// A bitmap font: glyph cells in an atlas, and how far each character advances.
#[derive(Clone, Debug, PartialEq)]
pub struct BitmapFont {
	pub atlas_width: u32,
	pub atlas_height: u32,
	pub cell_width: u32,
	pub cell_height: u32,
	/// The character in the first cell.
	pub first: u8,
	/// Advance width in pixels per character code.
	pub advances: [u8; 256],
}

/// Where one glyph's cell is in the atlas, in texture coordinates of the stored
/// (upside-down) image: left, right, top, bottom, as brush frames give them, so the
/// `top` value is the picture's bottom edge.
pub type GlyphUv = [f32; 4];

impl BitmapFont {
	pub fn parse(bytes: &[u8]) -> Result<BitmapFont, String> {
		let mut r = Reader::new(bytes);
		let atlas_width = r.u32("atlas width")?;
		let atlas_height = r.u32("atlas height")?;
		let cell_width = r.u32("cell width")?;
		let cell_height = r.u32("cell height")?;
		if atlas_width == 0 || atlas_height == 0 || cell_width == 0 || cell_height == 0 {
			return Err("font with an empty atlas or cell".into());
		}
		let first = r.u8("first character")?;
		let advances = r.array("advance widths")?;
		Ok(BitmapFont {
			atlas_width,
			atlas_height,
			cell_width,
			cell_height,
			first,
			advances,
		})
	}

	/// The cell holding `code`, or `None` below the first character or past the atlas.
	pub fn glyph_uv(&self, code: u8) -> Option<GlyphUv> {
		let index = u32::from(code.checked_sub(self.first)?);
		let columns = self.atlas_width / self.cell_width;
		let (column, row) = (index % columns, index / columns);
		let (x, y) = (column * self.cell_width, row * self.cell_height);
		if y + self.cell_height > self.atlas_height {
			return None;
		}
		let (w, h) = (self.atlas_width as f32, self.atlas_height as f32);
		// Row `row` from the picture's top is that far from the stored image's bottom.
		let top = (self.atlas_height - y - self.cell_height) as f32 / h;
		let bottom = (self.atlas_height - y) as f32 / h;
		Some([x as f32 / w, (x + self.cell_width) as f32 / w, top, bottom])
	}

	/// The width of `text` in pixels: the sum of its characters' advances.
	pub fn text_width(&self, text: &[u8]) -> u32 {
		text.iter()
			.map(|&c| u32::from(self.advances[usize::from(c)]))
			.sum()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn font_bytes() -> Vec<u8> {
		let mut out = Vec::new();
		for value in [512u32, 256, 22, 20] {
			out.extend_from_slice(&value.to_le_bytes());
		}
		out.push(32);
		out.extend((0..=255u8).map(|c| c % 16));
		out
	}

	#[test]
	fn parses_the_header_and_advances() {
		let font = BitmapFont::parse(&font_bytes()).unwrap();
		assert_eq!(
			(
				font.atlas_width,
				font.atlas_height,
				font.cell_width,
				font.cell_height
			),
			(512, 256, 22, 20)
		);
		assert_eq!(font.first, 32);
		assert_eq!(font.advances[b'A' as usize], 65 % 16);
		assert_eq!(font.text_width(b"AB"), 1 + 2);
	}

	#[test]
	fn glyphs_fill_rows_of_cells_from_the_pictures_top() {
		let font = BitmapFont::parse(&font_bytes()).unwrap();
		assert_eq!(font.glyph_uv(31), None);
		// The first character: top-left cell, so the stored image's bottom rows.
		assert_eq!(
			font.glyph_uv(b' '),
			Some([0.0, 22.0 / 512.0, 236.0 / 256.0, 1.0])
		);
		// 23 cells per row: character 32 + 23 starts the second row.
		assert_eq!(
			font.glyph_uv(32 + 23),
			Some([0.0, 22.0 / 512.0, 216.0 / 256.0, 236.0 / 256.0])
		);
		assert!(font.glyph_uv(255).is_some());
		// A cell past the atlas's last full row has no glyph.
		let mut short = font_bytes();
		short[4..8].copy_from_slice(&40u32.to_le_bytes());
		let short = BitmapFont::parse(&short).unwrap();
		assert!(short.glyph_uv(32 + 23 + 22).is_some());
		assert!(short.glyph_uv(32 + 23 * 2).is_none());
	}

	#[test]
	fn rejects_a_truncated_or_empty_font() {
		assert!(BitmapFont::parse(&font_bytes()[..100]).is_err());
		let mut zero = font_bytes();
		zero[8..12].copy_from_slice(&0u32.to_le_bytes());
		assert!(BitmapFont::parse(&zero).is_err());
	}
}
