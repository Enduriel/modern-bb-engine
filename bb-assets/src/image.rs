//! Decoding the game's PNG images into 8-bit RGBA pixels, ready to upload.
//!
//! The game and its mods ship 8-bit RGBA, RGB and paletted PNGs (no 16-bit or
//! interlaced ones as of 2026-10). Everything is normalized to RGBA8: palettes and
//! transparency chunks are expanded, 16-bit channels are cut to 8, and grey becomes
//! RGB.

use std::io::Cursor;

use png::{ColorType, Decoder, Limits, Transformations};

/// The largest decoded image accepted, in bytes: a 16384 × 16384 RGBA atlas.
const MAX_DECODED_BYTES: usize = 1 << 30;

/// An RGBA8 image. Rows are in file order (the first row of the PNG first).
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
	pub width: u32,
	pub height: u32,
	/// `width * height * 4` bytes.
	pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Image {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "Image({}x{})", self.width, self.height)
	}
}

/// Decode a PNG file into RGBA8.
pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
	let mut decoder = Decoder::new_with_limits(
		Cursor::new(bytes),
		Limits {
			bytes: MAX_DECODED_BYTES,
		},
	);
	decoder.set_transformations(Transformations::normalize_to_color8() | Transformations::ALPHA);
	let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
	let size = reader.output_buffer_size().ok_or("image too large")?;
	let mut pixels = vec![0; size];
	let frame = reader.next_frame(&mut pixels).map_err(|e| e.to_string())?;
	pixels.truncate(frame.buffer_size());
	let rgba = match frame.color_type {
		ColorType::Rgba => pixels,
		ColorType::GrayscaleAlpha => pixels
			.chunks_exact(2)
			.flat_map(|ga| [ga[0], ga[0], ga[0], ga[1]])
			.collect(),
		other => return Err(format!("unexpected decoded colour type {other:?}")),
	};
	Ok(Image {
		width: frame.width,
		height: frame.height,
		rgba,
	})
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;

	/// Encode a small PNG with the `png` crate's encoder.
	pub(crate) fn encode(
		width: u32,
		height: u32,
		color: ColorType,
		data: &[u8],
		palette: Option<(&[u8], &[u8])>,
	) -> Vec<u8> {
		let mut out = Vec::new();
		let mut encoder = png::Encoder::new(&mut out, width, height);
		encoder.set_color(color);
		encoder.set_depth(png::BitDepth::Eight);
		if let Some((palette, trns)) = palette {
			encoder.set_palette(palette.to_vec());
			encoder.set_trns(trns.to_vec());
		}
		let mut writer = encoder.write_header().unwrap();
		writer.write_image_data(data).unwrap();
		writer.finish().unwrap();
		out
	}

	#[test]
	fn rgba_passes_through() {
		let data = [1, 2, 3, 4, 5, 6, 7, 8];
		let image = decode_png(&encode(2, 1, ColorType::Rgba, &data, None)).unwrap();
		assert_eq!((image.width, image.height), (2, 1));
		assert_eq!(image.rgba, data);
	}

	#[test]
	fn rgb_gains_an_opaque_alpha() {
		let image = decode_png(&encode(
			1,
			2,
			ColorType::Rgb,
			&[10, 20, 30, 40, 50, 60],
			None,
		))
		.unwrap();
		assert_eq!(image.rgba, [10, 20, 30, 255, 40, 50, 60, 255]);
	}

	#[test]
	fn palette_entries_expand_with_their_transparency() {
		let palette = [255, 0, 0, 0, 0, 255];
		let trns = [128];
		let image = decode_png(&encode(
			2,
			1,
			ColorType::Indexed,
			&[0, 1],
			Some((&palette, &trns)),
		))
		.unwrap();
		assert_eq!(image.rgba, [255, 0, 0, 128, 0, 0, 255, 255]);
	}

	#[test]
	fn grey_becomes_rgb() {
		let image = decode_png(&encode(2, 1, ColorType::Grayscale, &[7, 200], None)).unwrap();
		assert_eq!(image.rgba, [7, 7, 7, 255, 200, 200, 200, 255]);
	}

	#[test]
	fn garbage_is_an_error() {
		assert!(decode_png(b"not a png").is_err());
	}
}
