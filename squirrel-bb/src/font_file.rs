//! What a font file is, read from its tables (std only): whether it's a TrueType,
//! OpenType or collection file, and its family's name, for `ModernEngine.registerFont`.

/// Whether `data` starts like a font file: TrueType, OpenType (CFF) or a collection.
pub(crate) fn is_font(data: &[u8]) -> bool {
	matches!(data.get(..4), Some([0, 1, 0, 0] | b"true" | b"OTTO" | b"ttcf"))
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
	Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
	Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// The family name of the (first) font in `data`: the typographic family (name 16),
/// else the family (name 1); Windows' UTF-16 records first, then Mac Roman.
pub(crate) fn family_name(data: &[u8]) -> Option<String> {
	let font = if data.get(..4)? == b"ttcf" { u32_at(data, 12)? as usize } else { 0 };
	let tables = u16_at(data, font + 4)? as usize;
	let name = (0..tables).find_map(|i| {
		let record = font + 12 + i * 16;
		(data.get(record..record + 4)? == b"name").then(|| u32_at(data, record + 8)).flatten()
	})? as usize;
	let count = u16_at(data, name + 2)? as usize;
	let strings = name + u16_at(data, name + 4)? as usize;
	let mut found: Vec<(u8, String)> = Vec::new();
	for i in 0..count {
		let r = name + 6 + i * 12;
		let (platform, encoding, id) = (u16_at(data, r)?, u16_at(data, r + 2)?, u16_at(data, r + 6)?);
		let (len, offset) = (u16_at(data, r + 8)? as usize, u16_at(data, r + 10)? as usize);
		let bytes = data.get(strings + offset..strings + offset + len)?;
		let text = match (platform, encoding) {
			(3, 0 | 1 | 10) | (0, _) => {
				let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
				String::from_utf16(&units).ok()?
			}
			(1, 0) => bytes.iter().map(|&b| b as char).collect(),
			_ => continue,
		};
		// Rank: typographic family before family, Windows before Mac.
		let rank = match (id, platform) {
			(16, 3 | 0) => 0,
			(16, _) => 1,
			(1, 3 | 0) => 2,
			(1, _) => 3,
			_ => continue,
		};
		found.push((rank, text));
	}
	found.sort();
	found.into_iter().next().map(|(_, name)| name).filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_font_files_family_is_read_from_its_name_table() {
		let otf = include_bytes!("../../bb-render/tests/fixtures/bb_test_han_serif.otf");
		assert!(is_font(otf));
		assert_eq!(family_name(otf).as_deref(), Some("BB Test Han Serif"));
		assert!(!is_font(b"PK\x03\x04 a zip"));
		assert_eq!(family_name(b"not a font at all"), None);
	}
}
