//! Mods' fonts on the UI page: what a font registered with
//! `ModernEngine.registerFont` adds to the page's style.
//!
//! Each registration adds an `@font-face` to every family of vanilla's `ui/fonts.css` it
//! stands in for ([`FontRegistration::ui_families`]), under that family's own name and
//! limited by `unicode-range` to the code points vanilla's fonts weren't made for (those
//! outside Windows-1252, of the registration's scripts if it names some). Chromium joins
//! the faces of one family into one font and, for each character, tries the faces whose
//! range has it, the last defined first, then the family's next face: vanilla's text keeps
//! vanilla's face, other scripts get the mod's font, and characters it lacks still fall
//! back to vanilla's face and then the system's fonts. So the faces are written lowest
//! priority first and, at the same priority, in registration order: a mod loaded later
//! overrides an earlier one, as on the map.
//!
//! The file is served from the game's files (`coui://<path>`), as the page's own fonts are.

use squirrel_bb::modern_engine::FontRegistration;

/// The `<style>` element the faces go in.
const STYLE_ID: &str = "bb-modern-engine-fonts";

/// One registration's faces, as the page takes them.
#[derive(Clone, Debug)]
struct Faces {
	priority: i64,
	css: String,
}

/// The faces of the fonts registered so far.
#[derive(Debug, Default)]
pub struct PageFonts {
	/// In registration order.
	faces: Vec<Faces>,
}

impl PageFonts {
	/// How many registrations it has.
	pub fn len(&self) -> usize {
		self.faces.len()
	}

	pub fn is_empty(&self) -> bool {
		self.faces.is_empty()
	}

	/// Add a registration's faces (none if it stands in for none of the page's families).
	pub fn add(&mut self, font: &FontRegistration) {
		self.faces.push(Faces {
			priority: font.priority,
			css: faces(font),
		});
	}

	/// The style sheet: every registration's faces, lowest priority first, then in
	/// registration order (the last defined is tried first).
	pub fn style_sheet(&self) -> String {
		let mut faces: Vec<&Faces> = self.faces.iter().collect();
		// Stable: registration order within a priority.
		faces.sort_by_key(|f| f.priority);
		faces.iter().map(|f| f.css.as_str()).collect()
	}

	/// The page script that puts [`PageFonts::style_sheet`] in place: in its own
	/// `<style>`, made last in `<head>` (after the page's and mods' style sheets, so its
	/// faces are tried first).
	pub fn page_script(&self) -> String {
		let css = crate::json::Json::Str(self.style_sheet()).to_text();
		format!(
			r#"(function (css) {{
	var style = document.getElementById('{STYLE_ID}');
	if (style === null) {{
		style = document.createElement('style');
		style.id = '{STYLE_ID}';
	}}
	style.textContent = css;
	document.head.appendChild(style);
	return true;
}})({css})"#
		)
	}
}

/// `font`'s `@font-face` rules, one per page family it stands in for.
fn faces(font: &FontRegistration) -> String {
	let families = font.ui_families();
	if families.is_empty() {
		return String::new();
	}
	let url = css_string(&format!("coui://{}", url_path(&font.path)));
	let range = font.ui_unicode_range();
	families
		.into_iter()
		.map(|family| {
			let family = css_string(family);
			format!("@font-face {{\n\tfont-family: {family};\n\tsrc: url({url});\n\tunicode-range: {range};\n}}\n")
		})
		.collect()
}

/// `text` as a quoted CSS string.
fn css_string(text: &str) -> String {
	let mut out = String::with_capacity(text.len() + 2);
	out.push('"');
	for c in text.chars() {
		match c {
			'"' | '\\' => {
				out.push('\\');
				out.push(c);
			}
			// A newline can't be in a CSS string as itself.
			'\n' | '\r' | '\x0c' => out.push_str(&format!("\\{:x} ", u32::from(c))),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// A game file's path as a URL path: each byte outside the unreserved characters (and
/// `/`) percent-escaped, as `coui://` URLs are decoded (`crate::coui::url_to_path`).
fn url_path(path: &str) -> String {
	let mut out = String::with_capacity(path.len());
	for byte in path.bytes() {
		match byte {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => out.push(byte as char),
			b => out.push_str(&format!("%{b:02X}")),
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn font(path: &str, stands_in_for: Option<&[&str]>, scripts: Option<&[&str]>, priority: i64) -> FontRegistration {
		FontRegistration {
			path: path.into(),
			data: Vec::new(),
			family: "Test".into(),
			stands_in_for: stands_in_for.map(|f| f.iter().map(|s| s.to_string()).collect()),
			scripts: scripts.map(|s| s.iter().map(|s| s.to_string()).collect()),
			priority,
		}
	}

	#[test]
	fn a_face_per_family_under_the_familys_own_name() {
		let mut fonts = PageFonts::default();
		fonts.add(&font("gfx/my mod/han.otf", Some(&["cinzel_bold_20", "Arsenal-Regular"]), Some(&["Han"]), 0));
		let css = fonts.style_sheet();
		assert_eq!(css.matches("@font-face").count(), 2, "{css}");
		assert!(css.contains("font-family: \"Arsenal-Regular\";") && css.contains("font-family: \"Cinzel-Bold\";"));
		assert!(css.contains("src: url(\"coui://gfx/my%20mod/han.otf\");"));
		assert_eq!(crate::coui::url_to_path("coui://gfx/my%20mod/han.otf").as_deref(), Some("gfx/my mod/han.otf"));
		assert!(css.contains("unicode-range: U+"));
	}

	#[test]
	fn the_highest_priority_and_then_the_latest_comes_last() {
		let mut fonts = PageFonts::default();
		fonts.add(&font("a.otf", None, None, 1));
		fonts.add(&font("b.otf", None, None, 0));
		fonts.add(&font("c.otf", None, None, 1));
		let css = fonts.style_sheet();
		let at = |name: &str| css.find(name).unwrap();
		assert!(at("b.otf") < at("a.otf") && at("a.otf") < at("c.otf"), "{css}");
		assert_eq!(css.matches("@font-face").count(), 18, "all six families each");
		assert!(fonts.page_script().contains("bb-modern-engine-fonts"));
	}

	#[test]
	fn a_font_for_no_family_adds_nothing() {
		let mut fonts = PageFonts::default();
		fonts.add(&font("a.otf", Some(&[]), None, 0));
		assert_eq!((fonts.len(), fonts.style_sheet()), (1, String::new()));
		assert_eq!(css_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\a d\"");
	}
}
