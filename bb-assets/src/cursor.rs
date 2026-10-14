//! Windows cursor files (`.cur`), which the original loads from `<install>/cursors/`
//! for its hardware cursor (`LoadCursorFromFileA`). The game's are 32×32 with an
//! alpha channel: a one-image icon directory whose image is a bottom-up 32-bit
//! bitmap followed by its AND mask, with the hotspot in the directory entry.

/// A cursor's picture and hotspot.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorImage {
	pub width: u32,
	pub height: u32,
	/// From the top left.
	pub hotspot: (u16, u16),
	/// RGBA, top row first.
	pub rgba: Vec<u8>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
	Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
	Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The first image of the cursor file `bytes`, if it is a 32-bit bitmap cursor.
pub fn parse_cursor(bytes: &[u8]) -> Option<CursorImage> {
	// The directory: reserved 0, type 2 (cursor), image count.
	if u16_at(bytes, 0)? != 0 || u16_at(bytes, 2)? != 2 || u16_at(bytes, 4)? == 0 {
		return None;
	}
	let hotspot = (u16_at(bytes, 10)?, u16_at(bytes, 12)?);
	let offset = u32_at(bytes, 18)? as usize;
	// The bitmap header: its height counts the AND mask too.
	let header = u32_at(bytes, offset)? as usize;
	let width = u32_at(bytes, offset + 4)?;
	let height = u32_at(bytes, offset + 8)? / 2;
	if u16_at(bytes, offset + 14)? != 32 || width == 0 || height == 0 {
		return None;
	}
	let pixels = bytes.get(offset + header..offset + header + (width * height * 4) as usize)?;
	let row = (width * 4) as usize;
	let mut rgba = Vec::with_capacity(pixels.len());
	for y in (0..height as usize).rev() {
		for bgra in pixels[y * row..(y + 1) * row].chunks_exact(4) {
			rgba.extend_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
		}
	}
	Some(CursorImage {
		width,
		height,
		hotspot,
		rgba,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A 2×2 cursor, hotspot (1, 0): red top left, the rest blue, bottom row first.
	fn cursor() -> Vec<u8> {
		let mut b = vec![0, 0, 2, 0, 1, 0];
		b.extend_from_slice(&[2, 2, 0, 0, 1, 0, 0, 0]);
		b.extend_from_slice(&(40u32 + 16 + 8).to_le_bytes());
		b.extend_from_slice(&22u32.to_le_bytes());
		b.extend_from_slice(&40u32.to_le_bytes());
		b.extend_from_slice(&2u32.to_le_bytes());
		b.extend_from_slice(&4u32.to_le_bytes());
		b.extend_from_slice(&1u16.to_le_bytes());
		b.extend_from_slice(&32u16.to_le_bytes());
		b.resize(22 + 40, 0);
		let blue = [255, 0, 0, 255];
		let red = [0, 0, 255, 255];
		for p in [blue, blue, red, blue] {
			b.extend_from_slice(&p);
		}
		b.extend_from_slice(&[0; 8]);
		b
	}

	#[test]
	fn reads_a_32_bit_cursor_top_down() {
		let c = parse_cursor(&cursor()).expect("a cursor");
		assert_eq!((c.width, c.height, c.hotspot), (2, 2, (1, 0)));
		assert_eq!(&c.rgba[..4], &[255, 0, 0, 255], "top left: red");
		assert_eq!(&c.rgba[4..8], &[0, 0, 255, 255]);
		assert_eq!(parse_cursor(b"not a cursor"), None);
	}
}
