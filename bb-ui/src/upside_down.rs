//! Pictures turned upside down. Coherent draws a background image that tiles (one
//! that doesn't cover its box from where it is placed, and repeats) with every tile
//! upside down; the page asks for such a picture as `coui://…png?bb-upside-down`
//! (`host/tiles.js`).

use bb_assets::decode_png;

/// The query that asks `coui://` for a picture upside down.
pub const QUERY: &str = "bb-upside-down";

/// Chunks that say how to read the pixels' colours: they hold for the turned picture
/// as they did for the original.
const COLOUR_CHUNKS: [&[u8; 4]; 4] = [b"gAMA", b"cHRM", b"sRGB", b"iCCP"];

/// Whether `url` asks for its picture upside down.
pub fn asked_for(url: &str) -> bool {
	let Some((_, query)) = url.split('#').next().unwrap_or("").split_once('?') else {
		return false;
	};
	query.split('&').any(|part| part == QUERY)
}

/// The PNG `png` with its rows in reverse order, as a PNG.
pub fn upside_down(png: &[u8]) -> Result<Vec<u8>, String> {
	let image = decode_png(png)?;
	let row = image.width as usize * 4;
	let mut rgba = Vec::with_capacity(image.rgba.len());
	if row > 0 {
		for line in image.rgba.chunks_exact(row).rev() {
			rgba.extend_from_slice(line);
		}
	}
	let mut out = Vec::new();
	crate::png::write_rgba_with(
		&mut out,
		image.width,
		image.height,
		&rgba,
		&colour_chunks(png),
	)
	.map_err(|e| e.to_string())?;
	Ok(out)
}

/// The colour chunks of the PNG `png`, in file order.
fn colour_chunks(png: &[u8]) -> Vec<([u8; 4], &[u8])> {
	let mut chunks = Vec::new();
	let mut at = 8;
	while let Some(header) = png.get(at..at + 8) {
		let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
		let kind = [header[4], header[5], header[6], header[7]];
		let Some(data) = png.get(at + 8..at + 8 + length) else {
			break;
		};
		if &kind == b"IDAT" {
			break;
		}
		if COLOUR_CHUNKS.contains(&&kind) {
			chunks.push((kind, data));
		}
		at += 12 + length;
	}
	chunks
}

#[cfg(test)]
mod tests {
	use super::{asked_for, colour_chunks, upside_down};
	use bb_assets::decode_png;

	/// A 1×3 picture, red over green over blue, with a gamma chunk.
	fn picture() -> Vec<u8> {
		let mut png = Vec::new();
		let rgba = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0];
		crate::png::write_rgba_with(&mut png, 1, 3, &rgba, &[(*b"gAMA", &[0, 0, 0xB1, 0x8F])])
			.unwrap();
		png
	}

	#[test]
	fn turns_the_rows_around_and_keeps_the_colour_chunks() {
		let turned = upside_down(&picture()).unwrap();
		let image = decode_png(&turned).unwrap();
		assert_eq!((image.width, image.height), (1, 3));
		assert_eq!(image.rgba, [0, 0, 255, 0, 0, 255, 0, 128, 255, 0, 0, 255]);
		assert_eq!(
			colour_chunks(&turned),
			[(*b"gAMA", &[0u8, 0, 0xB1, 0x8F][..])]
		);
	}

	#[test]
	fn knows_the_query() {
		assert!(asked_for(
			"coui://gfx/ui/skin/button_04_default.png?bb-upside-down"
		));
		assert!(asked_for("coui://gfx/a.png?x=1&bb-upside-down#top"));
		assert!(!asked_for("coui://gfx/a.png"));
		assert!(!asked_for("coui://gfx/a.png#bb-upside-down"));
		assert!(!asked_for("coui://gfx/a.png?bb-upside-down-not"));
	}

	#[test]
	fn a_file_that_is_no_png_is_an_error() {
		assert!(upside_down(b"GIF89a").is_err());
	}
}
