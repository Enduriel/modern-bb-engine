//! The outline fonts engine-drawn text falls back to: text a bitmap font can't
//! draw (see [`decode`]) is shaped and drawn from these.
//!
//! The database holds, in the order a run of text looks for a font:
//! 1. fonts mods register (`ModernEngine.registerFont`), each standing in for some of
//!    vanilla's bitmap fonts, for some scripts, at a priority (higher first; at equal
//!    priority the later registration first, so a later mod overrides an earlier one);
//! 2. the game's own TTF for the bitmap font (`gfx/fonts/cinzel/`: Cinzel);
//! 3. the system's fonts, through cosmic-text's fallback (fontdb finds them on every
//!    system).
//!
//! A text is split into runs by Unicode script ([`FontDb::runs`]); each run gets the
//! first registered font that stands in for its bitmap font, covers its script and has
//! its characters, else the game's own. Characters still missing come from the system's
//! fonts. Registering bumps [`FontDb::revision`], so text shaped before is shaped again.

pub mod decode;
pub mod shaper;

use std::ops::Range;

use cosmic_text::{Attrs, Family, FontSystem, Weight, fontdb};
use unicode_script::{Script, UnicodeScript};

/// One of vanilla's bitmap fonts, and the outline font it was made from.
struct GameFont {
	/// The bitmap font's name (`gfx/fonts/<name>.fnt`).
	bitmap: &'static str,
	family: &'static str,
	weight: u16,
	/// The pixel size it was made at (its name says it).
	size: f32,
}

const GAME_FONTS: [GameFont; 3] = [
	GameFont { bitmap: "cinzel_20", family: "Cinzel", weight: 400, size: 20.0 },
	GameFont { bitmap: "cinzel_bold_20", family: "Cinzel", weight: 700, size: 20.0 },
	GameFont { bitmap: "cinzel_bold_100", family: "Cinzel", weight: 700, size: 100.0 },
];

/// A font a mod registers: its file, and where it stands in.
#[derive(Clone, Debug, Default)]
pub struct Registration {
	/// Its path in the game's files (for messages).
	pub path: String,
	pub data: Vec<u8>,
	/// The bitmap fonts it stands in for (`None`: all of them).
	pub stands_in_for: Option<Vec<String>>,
	/// The scripts it's for, by Unicode name (`"Han"`; `None`: all of them).
	pub scripts: Option<Vec<String>>,
	pub priority: i64,
}

/// A registered font, checked.
struct Rule {
	family: String,
	weight: Weight,
	face: fontdb::ID,
	stands_in_for: Option<Vec<String>>,
	scripts: Option<Vec<Script>>,
	priority: i64,
	order: usize,
}

/// A run of text and the font it's drawn in.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
	/// Byte range in the text.
	pub range: Range<usize>,
	pub family: String,
	pub weight: u16,
}

/// The outline fonts, and the rules for choosing among them.
pub struct FontDb {
	system: FontSystem,
	rules: Vec<Rule>,
	revision: u64,
	/// Whether the system's fonts are wanted, and whether they're loaded yet (on the
	/// first text that needs them: vanilla's text never does).
	system_fonts: bool,
	system_loaded: bool,
}

impl FontDb {
	/// The game's own TTFs (`gfx/fonts/**.ttf`, their bytes), and the system's fonts if
	/// `system_fonts` (off for tests that must not depend on the machine).
	pub fn new(game_fonts: Vec<Vec<u8>>, system_fonts: bool) -> FontDb {
		let mut db = fontdb::Database::new();
		for data in game_fonts {
			db.load_font_data(data);
		}
		FontDb {
			system: FontSystem::new_with_locale_and_db("en-US".to_string(), db),
			rules: Vec::new(),
			revision: 0,
			system_fonts,
			system_loaded: false,
		}
	}

	/// Load the system's fonts, the first time a text needs the outline fonts.
	pub fn load_system_fonts(&mut self) {
		if self.system_fonts && !self.system_loaded {
			self.system_loaded = true;
			self.system.db_mut().load_system_fonts();
		}
	}

	/// Add a mod's font; its family's name, or why it can't be used.
	pub fn register(&mut self, registration: Registration) -> Result<String, String> {
		let path = &registration.path;
		if let Some(fonts) = &registration.stands_in_for {
			if let Some(unknown) = fonts.iter().find(|f| *f != "*" && !GAME_FONTS.iter().any(|g| g.bitmap == *f)) {
				return Err(format!("{path}: no bitmap font {unknown:?} (one of {:?})", GAME_FONTS.map(|g| g.bitmap)));
			}
		}
		let scripts = match &registration.scripts {
			None => None,
			Some(names) => Some(
				names
					.iter()
					.map(|n| Script::from_full_name(n).ok_or_else(|| format!("{path}: no Unicode script {n:?}")))
					.collect::<Result<Vec<_>, _>>()?,
			),
		};
		let ids = self.system.db_mut().load_font_source(fontdb::Source::Binary(std::sync::Arc::new(registration.data)));
		let face = *ids.first().ok_or_else(|| format!("{path}: not a font"))?;
		let info = self.system.db().face(face).ok_or_else(|| format!("{path}: not a font"))?;
		let family = info.families.first().map(|(name, _)| name.clone()).ok_or_else(|| format!("{path}: a font without a family name"))?;
		let weight = info.weight;
		let stands_in_for = registration.stands_in_for.filter(|f| !f.iter().any(|f| f == "*"));
		self.rules.push(Rule {
			family: family.clone(),
			weight,
			face,
			stands_in_for,
			scripts,
			priority: registration.priority,
			order: self.rules.len(),
		});
		self.rules.sort_by(|a, b| b.priority.cmp(&a.priority).then(b.order.cmp(&a.order)));
		self.revision += 1;
		Ok(family)
	}

	/// Bumped by every registration: what was shaped before may draw differently now.
	pub fn revision(&self) -> u64 {
		self.revision
	}

	/// The pixel size the outline font draws at in place of the bitmap font `bitmap`.
	pub fn size(bitmap: &str) -> f32 {
		GAME_FONTS.iter().find(|g| g.bitmap == bitmap).map_or(20.0, |g| g.size)
	}

	/// `text` split into runs by script, each with the font it's drawn in where the
	/// game uses the bitmap font `bitmap`. A character common to every script (spaces,
	/// digits, punctuation) or a mark goes with the run around it.
	pub fn runs(&mut self, bitmap: &str, text: &str) -> Vec<Run> {
		let game = GAME_FONTS.iter().find(|g| g.bitmap == bitmap).unwrap_or(&GAME_FONTS[1]);
		let mut runs: Vec<(Range<usize>, Script)> = Vec::new();
		for (at, c) in text.char_indices() {
			let end = at + c.len_utf8();
			let script = c.script();
			let neutral = matches!(script, Script::Common | Script::Inherited | Script::Unknown);
			match runs.last_mut() {
				Some((range, last)) if neutral || *last == script => range.end = end,
				Some((range, last)) if matches!(last, Script::Common | Script::Inherited | Script::Unknown) => {
					range.end = end;
					*last = script;
				}
				_ => runs.push((at..end, script)),
			}
		}
		runs.into_iter()
			.map(|(range, script)| {
				let chars: Vec<char> = text[range.clone()].chars().filter(|c| !c.is_whitespace()).collect();
				let rule = self.rules.iter().find(|r| {
					r.stands_in_for.as_ref().is_none_or(|f| f.iter().any(|f| f == bitmap))
						&& r.scripts.as_ref().is_none_or(|s| s.contains(&script))
						&& has_chars(&mut self.system, r.face, r.weight, &chars)
				});
				match rule {
					Some(r) => Run { range, family: r.family.clone(), weight: r.weight.0 },
					None => Run { range, family: game.family.to_string(), weight: game.weight },
				}
			})
			.collect()
	}

	/// The attributes cosmic-text shapes `run` with.
	pub fn attrs(run: &Run) -> Attrs<'_> {
		Attrs::new().family(Family::Name(&run.family)).weight(Weight(run.weight))
	}

	/// The font system, for shaping and rasterizing.
	pub fn system(&mut self) -> &mut FontSystem {
		&mut self.system
	}
}

/// Whether the face `id` has a glyph for every one of `chars`.
fn has_chars(system: &mut FontSystem, id: fontdb::ID, weight: Weight, chars: &[char]) -> bool {
	let Some(font) = system.get_font(id, weight) else { return false };
	let charmap = font.as_swash().charmap();
	chars.iter().all(|c| charmap.map(*c) != 0)
}

#[cfg(test)]
mod tests {
	use super::*;

	const HAN_FONT: &[u8] = include_bytes!("../../tests/fixtures/bb_test_han_serif.otf");

	fn han(stands_in_for: Option<&[&str]>, scripts: Option<&[&str]>, priority: i64) -> Registration {
		Registration {
			path: "gfx/test/bb_test_han_serif.otf".into(),
			data: HAN_FONT.to_vec(),
			stands_in_for: stands_in_for.map(|f| f.iter().map(|s| s.to_string()).collect()),
			scripts: scripts.map(|s| s.iter().map(|s| s.to_string()).collect()),
			priority,
		}
	}

	fn families(db: &mut FontDb, bitmap: &str, text: &str) -> Vec<(String, String)> {
		db.runs(bitmap, text).into_iter().map(|r| (text[r.range].to_string(), r.family)).collect()
	}

	#[test]
	fn text_splits_by_script_and_neutral_characters_join_their_run() {
		let mut db = FontDb::new(Vec::new(), false);
		let runs = families(&mut db, "cinzel_bold_20", "白石镇 (2) Town");
		assert_eq!(runs, [("白石镇 (2) ".to_string(), "Cinzel".to_string()), ("Town".to_string(), "Cinzel".to_string())]);
	}

	#[test]
	fn a_registered_font_draws_its_scripts_where_it_stands_in() {
		let mut db = FontDb::new(Vec::new(), false);
		let family = db.register(han(Some(&["cinzel_bold_20", "cinzel_bold_100"]), Some(&["Han"]), 0)).expect("a font");
		assert_eq!(family, "BB Test Han Serif");
		assert_eq!(db.revision(), 1);
		let text = "黑森林 Forest";
		assert_eq!(families(&mut db, "cinzel_bold_100", text)[0].1, "BB Test Han Serif");
		assert_eq!(families(&mut db, "cinzel_bold_100", text)[1].1, "Cinzel", "Latin keeps the game's font");
		assert_eq!(families(&mut db, "cinzel_20", text)[0].1, "Cinzel", "not where it doesn't stand in");
		// A character the font lacks: the run goes to the game's font (and the system's).
		assert_eq!(families(&mut db, "cinzel_bold_20", "龍")[0].1, "Cinzel");
	}

	#[test]
	fn mistakes_are_refused_with_a_reason() {
		let mut db = FontDb::new(Vec::new(), false);
		let unknown_font = db.register(han(Some(&["cinzel_bold_30"]), None, 0)).unwrap_err();
		assert!(unknown_font.contains("cinzel_bold_30"), "{unknown_font}");
		let unknown_script = db.register(han(None, Some(&["Klingon"]), 0)).unwrap_err();
		assert!(unknown_script.contains("Klingon"), "{unknown_script}");
		let mut not_a_font = han(None, None, 0);
		not_a_font.data = b"not a font".to_vec();
		assert!(db.register(not_a_font).unwrap_err().contains("not a font"));
		assert_eq!(db.revision(), 0, "nothing registered");
	}

	#[test]
	fn higher_priority_then_the_later_registration_comes_first() {
		let mut db = FontDb::new(Vec::new(), false);
		db.register(han(None, Some(&["Han"]), 0)).unwrap();
		db.register(han(None, Some(&["Han"]), 5)).unwrap();
		db.register(han(None, Some(&["Han"]), 0)).unwrap();
		let order: Vec<(i64, usize)> = db.rules.iter().map(|r| (r.priority, r.order)).collect();
		assert_eq!(order, [(5, 1), (0, 2), (0, 0)]);
	}
}
