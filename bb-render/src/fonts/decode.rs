//! What a script's text bytes stand for, for the engine-drawn text.
//!
//! Vanilla draws engine text one glyph per byte through its bitmap fonts' Windows-1252
//! layout (verified against the original: a label set to UTF-8 `é` shows `Ã©`). Ours
//! decodes the bytes as UTF-8 where they're valid UTF-8 with a character beyond ASCII,
//! and keeps them as Windows-1252 otherwise (a divergence, `docs/differences.md`: such
//! text shows as its author meant it). Text whose characters all have a Windows-1252
//! byte still draws with the bitmap font, so vanilla's text draws exactly as before;
//! any other goes to the outline fonts (see [`super::FontDb`]).

use std::borrow::Cow;

/// How a text is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineText<'a> {
	/// With the bitmap font, one glyph per byte (Windows-1252), as vanilla draws it.
	Bitmap(Cow<'a, [u8]>),
	/// Characters the bitmap font lacks: shaped and drawn from outline fonts.
	Shaped(String),
}

/// How the script text `bytes` is drawn.
pub fn engine_text(bytes: &[u8]) -> EngineText<'_> {
	match std::str::from_utf8(bytes) {
		Ok(text) if !text.is_ascii() => match to_cp1252(text) {
			Some(bytes) => EngineText::Bitmap(Cow::Owned(bytes)),
			None => EngineText::Shaped(text.to_string()),
		},
		_ => EngineText::Bitmap(Cow::Borrowed(bytes)),
	}
}

/// The characters Windows-1252 gives 0x80–0x9F (`None`: undefined, kept as the C1
/// control of the same value).
const HIGH: [Option<char>; 32] = [
	Some('€'), None, Some('‚'), Some('ƒ'), Some('„'), Some('…'), Some('†'), Some('‡'),
	Some('ˆ'), Some('‰'), Some('Š'), Some('‹'), Some('Œ'), None, Some('Ž'), None,
	None, Some('‘'), Some('’'), Some('“'), Some('”'), Some('•'), Some('–'), Some('—'),
	Some('˜'), Some('™'), Some('š'), Some('›'), Some('œ'), None, Some('ž'), Some('Ÿ'),
];

/// The character the Windows-1252 byte `b` stands for.
pub fn cp1252_char(b: u8) -> char {
	match b {
		0x80..=0x9f => HIGH[usize::from(b - 0x80)].unwrap_or(char::from(b)),
		_ => char::from(b),
	}
}

/// `text` in Windows-1252, if every character has a byte there.
pub fn to_cp1252(text: &str) -> Option<Vec<u8>> {
	text.chars()
		.map(|c| match u32::from(c) {
			0..=0x7f | 0xa0..=0xff => Some(c as u8),
			_ => HIGH.iter().position(|h| *h == Some(c)).map(|i| 0x80 + i as u8),
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn vanillas_text_stays_on_the_bitmap_font_as_its_bytes() {
		assert_eq!(engine_text(b"Lorrested"), EngineText::Bitmap(Cow::Borrowed(&b"Lorrested"[..])));
		// Windows-1252 bytes that aren't UTF-8: kept as they are.
		assert_eq!(engine_text(b"Caf\xe9 \x80"), EngineText::Bitmap(Cow::Borrowed(&b"Caf\xe9 \x80"[..])));
	}

	#[test]
	fn utf8_text_draws_as_written() {
		// Characters Windows-1252 has: the bitmap font, with their Windows-1252 bytes.
		assert_eq!(engine_text("Café €".as_bytes()), EngineText::Bitmap(Cow::Owned(b"Caf\xe9 \x80".to_vec())));
		// Characters it hasn't: the outline fonts.
		assert_eq!(engine_text("白石镇".as_bytes()), EngineText::Shaped("白石镇".to_string()));
		assert_eq!(engine_text("Café 白".as_bytes()), EngineText::Shaped("Café 白".to_string()));
	}

	#[test]
	fn windows_1252_round_trips() {
		for b in 0..=255u8 {
			let c = cp1252_char(b);
			if HIGH.get(usize::from(b.wrapping_sub(0x80))).is_some_and(Option::is_none) && (0x80..=0x9f).contains(&b) {
				continue;
			}
			assert_eq!(to_cp1252(&c.to_string()), Some(vec![b]), "byte {b:#x}");
		}
	}
}
