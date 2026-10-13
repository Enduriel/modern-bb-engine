//! Saving rendered images as PNG files (for tests and comparisons).

use std::io::Write;

use bb_assets::Image;

/// Encode `image` as an RGBA PNG.
pub fn write_png(out: impl Write, image: &Image) -> Result<(), String> {
	let mut encoder = png::Encoder::new(out, image.width, image.height);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
	writer
		.write_image_data(&image.rgba)
		.map_err(|e| e.to_string())?;
	writer.finish().map_err(|e| e.to_string())
}

/// Save `image` as a PNG at `path`.
pub fn save_png(path: impl AsRef<std::path::Path>, image: &Image) -> Result<(), String> {
	let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
	write_png(std::io::BufWriter::new(file), image)
}
