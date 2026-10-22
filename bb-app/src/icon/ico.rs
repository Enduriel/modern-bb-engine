//! The `.ico` file's images: read for the window (the pixels, [`Image::rgba`]), for
//! `bb.exe`'s resources (the images as stored, `res`) and for the macOS bundle's `.icns`
//! (`bb-package`). Shared with `build.rs` and `bb-package` by path, so std only.
//!
//! The game's icon has 32-bit images only (16, 32 and 48 pixels: an alpha channel, the
//! AND mask unused), which is all this reads; a PNG image (Vista's 256-pixel ones) is
//! refused.

/// One of the file's images.
pub struct Image<'a> {
	/// Its size in pixels (square).
	pub size: u32,
	/// Its `ICONDIRENTRY` fields as the file has them (`bColorCount`, `wPlanes`,
	/// `wBitCount`), which the group resource repeats.
	pub colors: u8,
	pub planes: u16,
	pub bits: u16,
	/// The image as stored: a `BITMAPINFOHEADER`, its pixels and its AND mask.
	pub data: &'a [u8],
}

/// The images of the `.ico` file `file`, as it orders them.
pub fn images(file: &[u8]) -> Result<Vec<Image<'_>>, String> {
	let u16_at = |at: usize| file.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
	let u32_at = |at: usize| file.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
	if u16_at(0) != Some(0) || u16_at(2) != Some(1) {
		return Err("not an .ico file".into());
	}
	let count = u16_at(4).ok_or("no image count")? as usize;
	let mut images = Vec::with_capacity(count);
	for i in 0..count {
		let entry = 6 + 16 * i;
		if file.len() < entry + 16 {
			return Err("short directory".into());
		}
		let (width, height) = (file[entry], file[entry + 1]);
		let size = u32_at(entry + 8).unwrap_or(0) as usize;
		let offset = u32_at(entry + 12).unwrap_or(0) as usize;
		let data = file.get(offset..offset + size).ok_or("an image past the file's end")?;
		if width != height {
			return Err(format!("image {i} isn't square"));
		}
		let image = Image {
			size: if width == 0 { 256 } else { u32::from(width) },
			colors: file[entry + 2],
			planes: u16_at(entry + 4).unwrap_or(0),
			bits: u16_at(entry + 6).unwrap_or(0),
			data,
		};
		image.rgba()?;
		images.push(image);
	}
	Ok(images)
}

impl Image<'_> {
	/// Its pixels, top row first, as straight (not premultiplied) RGBA.
	pub fn rgba(&self) -> Result<Vec<u8>, String> {
		let data = self.data;
		let u16_at = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
		let i32_at = |at: usize| i32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
		if data.len() < 40 || i32_at(0) != 40 {
			return Err(format!("the {0}x{0} image isn't a bitmap (a PNG one?)", self.size));
		}
		let size = self.size as usize;
		// The height counts the AND mask too.
		if i32_at(4) != size as i32 || i32_at(8) != 2 * size as i32 || u16_at(14) != 32 || i32_at(16) != 0 {
			return Err(format!("the {0}x{0} image isn't an uncompressed 32-bit one", self.size));
		}
		let pixels = data.get(40..40 + size * size * 4).ok_or("short image")?;
		let mut rgba = Vec::with_capacity(size * size * 4);
		// Bottom row first, each pixel BGRA.
		for row in pixels.chunks_exact(size * 4).rev() {
			for bgra in row.chunks_exact(4) {
				rgba.extend_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
			}
		}
		Ok(rgba)
	}
}
