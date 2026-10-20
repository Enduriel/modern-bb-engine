//! The OpenType container (`sfnt`): a font as its tables, read and written whole.

use std::collections::BTreeMap;

/// A font's version tag for TrueType outlines.
pub const TRUETYPE: u32 = 0x0001_0000;
/// A font's version tag for CFF outlines (`OTTO`).
pub const CFF: u32 = 0x4F54_544F;

/// A font: its version tag and its tables by tag.
pub struct Font {
	pub version: u32,
	pub tables: BTreeMap<[u8; 4], Vec<u8>>,
}

/// The big-endian `u16` at `at`.
pub fn u16_at(data: &[u8], at: usize) -> Option<u16> {
	Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

/// The big-endian `u32` at `at`.
pub fn u32_at(data: &[u8], at: usize) -> Option<u32> {
	Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

impl Font {
	/// The font in `data`: a single TrueType or CFF font (not a collection, nor WOFF).
	pub fn read(data: &[u8]) -> Option<Font> {
		let version = u32_at(data, 0)?;
		if version != TRUETYPE && version != CFF {
			return None;
		}
		let count = u16_at(data, 4)? as usize;
		let mut tables = BTreeMap::new();
		for i in 0..count {
			let record = 12 + i * 16;
			let tag: [u8; 4] = data.get(record..record + 4)?.try_into().ok()?;
			let offset = u32_at(data, record + 8)? as usize;
			let length = u32_at(data, record + 12)? as usize;
			tables.insert(tag, data.get(offset..offset.checked_add(length)?)?.to_vec());
		}
		Some(Font { version, tables })
	}

	/// The table `tag`.
	pub fn table(&self, tag: &[u8; 4]) -> Option<&[u8]> {
		self.tables.get(tag).map(Vec::as_slice)
	}

	/// The font as a file: tables in tag order, each 4-byte aligned, with their
	/// checksums and the `head` table's whole-font checksum adjustment.
	pub fn write(&self) -> Vec<u8> {
		let count = self.tables.len();
		let mut power = 1;
		while power * 2 <= count {
			power *= 2;
		}
		let search_range = power * 16;
		let mut out = Vec::new();
		out.extend_from_slice(&self.version.to_be_bytes());
		out.extend_from_slice(&(count as u16).to_be_bytes());
		out.extend_from_slice(&(search_range as u16).to_be_bytes());
		out.extend_from_slice(&(power.trailing_zeros() as u16).to_be_bytes());
		out.extend_from_slice(&((count * 16 - search_range) as u16).to_be_bytes());
		let mut offset = 12 + count * 16;
		let mut head_at = None;
		for (tag, data) in &self.tables {
			let mut data = data.clone();
			if tag == b"head" && data.len() >= 12 {
				data[8..12].fill(0);
				head_at = Some(offset);
			}
			out.extend_from_slice(tag);
			out.extend_from_slice(&checksum(&data).to_be_bytes());
			out.extend_from_slice(&(offset as u32).to_be_bytes());
			out.extend_from_slice(&(data.len() as u32).to_be_bytes());
			offset += data.len().next_multiple_of(4);
		}
		for (tag, data) in &self.tables {
			let start = out.len();
			out.extend_from_slice(data);
			if tag == b"head" && data.len() >= 12 {
				out[start + 8..start + 12].fill(0);
			}
			out.resize(out.len().next_multiple_of(4), 0);
		}
		if let Some(at) = head_at {
			let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
			out[at + 8..at + 12].copy_from_slice(&adjustment.to_be_bytes());
		}
		out
	}
}

/// The OpenType checksum: the sum of big-endian `u32`s, zero-padded.
fn checksum(data: &[u8]) -> u32 {
	data.chunks(4).fold(0u32, |sum, chunk| {
		let mut word = [0; 4];
		word[..chunk.len()].copy_from_slice(chunk);
		sum.wrapping_add(u32::from_be_bytes(word))
	})
}

#[cfg(test)]
mod tests {
	use super::{Font, TRUETYPE, checksum};

	#[test]
	fn reads_back_what_it_wrote() {
		let mut font = Font { version: TRUETYPE, tables: Default::default() };
		font.tables.insert(*b"head", vec![1; 54]);
		font.tables.insert(*b"cmap", vec![2, 3, 4]);
		font.tables.insert(*b"glyf", vec![5; 8]);
		let file = font.write();
		assert_eq!(file.len() % 4, 0);
		let back = Font::read(&file).unwrap();
		assert_eq!(back.table(b"cmap"), Some(&[2u8, 3, 4][..]));
		assert_eq!(back.table(b"glyf"), Some(&[5u8; 8][..]));
		// The whole font sums to the magic number.
		assert_eq!(checksum(&file), 0xB1B0_AFBA);
		// Header: 3 tables, search range 32, entry selector 1, range shift 16.
		assert_eq!(&file[4..12], &[0, 3, 0, 32, 0, 1, 0, 16]);
	}

	#[test]
	fn takes_only_single_fonts() {
		assert!(Font::read(b"ttcf\0\x01\0\0\0\0\0\x01").is_none());
		assert!(Font::read(b"wOFF\0\x01\0\0\0\0\0\0").is_none());
		assert!(Font::read(&[0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]).is_none(), "a table past the end");
	}
}
