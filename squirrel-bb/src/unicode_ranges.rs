//! The code points a registered font is for on the UI's page, as CSS
//! `unicode-range`s: every one vanilla's fonts weren't made for (outside Windows-1252,
//! the engine's text encoding), so the game's own fonts keep the text they always drew;
//! of those, only the named Unicode scripts' and the punctuation and marks they share
//! (`Common`, `Inherited`: CJK punctuation, say) if it names some.

use std::ops::RangeInclusive;

use unicode_script::{Script, UnicodeScript};

/// The characters Windows-1252 gives 0x80–0x9F (as `bb_render::fonts::decode`'s table).
const CP1252_HIGH: [char; 27] = [
	'€', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', 'Ž', '‘', '’', '“', '”', '•', '–', '—', '˜', '™',
	'š', '›', 'œ', 'ž', 'Ÿ',
];

/// Whether Windows-1252 has `c`.
fn in_cp1252(c: char) -> bool {
	// The bytes it leaves undefined decode to the C1 controls of the same value.
	matches!(u32::from(c), 0..=0x7f | 0x81 | 0x8d | 0x8f | 0x90 | 0x9d | 0xa0..=0xff) || CP1252_HIGH.contains(&c)
}

/// The code points outside Windows-1252 of `scripts` (and of `Common` and `Inherited`;
/// `None`: of every script), as sorted, disjoint ranges.
pub fn code_point_ranges(scripts: Option<&[Script]>) -> Vec<RangeInclusive<u32>> {
	let wanted = |c: char| {
		let script = c.script();
		!in_cp1252(c)
			&& scripts.is_none_or(|s| s.contains(&script) || matches!(script, Script::Common | Script::Inherited))
	};
	let mut ranges: Vec<RangeInclusive<u32>> = Vec::new();
	for c in (0..=u32::from(char::MAX)).filter_map(char::from_u32).filter(|&c| wanted(c)) {
		let c = u32::from(c);
		match ranges.last_mut() {
			// Across the surrogates too: no character there to draw.
			Some(last) if *last.end() + 1 == c || (*last.end() == 0xd7ff && c == 0xe000) => {
				*last = *last.start()..=c;
			}
			_ => ranges.push(c..=c),
		}
	}
	ranges
}

/// `ranges` as a CSS `unicode-range` value (`U+4E00-9FFF, U+3007`).
pub fn css_unicode_range(ranges: &[RangeInclusive<u32>]) -> String {
	let range = |r: &RangeInclusive<u32>| match r.start() == r.end() {
		true => format!("U+{:X}", r.start()),
		false => format!("U+{:X}-{:X}", r.start(), r.end()),
	};
	ranges.iter().map(range).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn by_default_everything_vanilla_fonts_were_not_made_for() {
		let ranges = code_point_ranges(None);
		let has = |c: char| ranges.iter().any(|r| r.contains(&u32::from(c)));
		for c in ['A', 'é', 'ÿ', '€', '—', 'Ÿ', ' '] {
			assert!(!has(c), "{c:?} is the game fonts'");
		}
		for c in ['中', 'я', 'Ā', '\u{81}', '😀'] {
			assert_eq!(has(c), c != '\u{81}', "{c:?}");
		}
		assert_eq!(ranges.last(), Some(&(0x2123..=0x10ffff)), "one range from past ™ to the end");
	}

	#[test]
	fn a_scripts_code_points_only() {
		let ranges = code_point_ranges(Some(&[Script::Han]));
		let has = |c: char| ranges.iter().any(|r| r.contains(&u32::from(c)));
		assert!(has('中') && has('〇') && has('、') && !has('A') && !has('あ') && !has('—'));
		assert!(has('\u{20000}') && !has('\u{ffff}'), "CJK extensions, not what no script has");
		assert_eq!(css_unicode_range(&[0x41..=0x41, 0x4e00..=0x9fff]), "U+41, U+4E00-9FFF");
	}
}
