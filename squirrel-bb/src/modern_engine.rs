//! `ModernEngine`: the root table only this engine defines, so a mod can detect it
//! (`"ModernEngine" in ::getroottable()`) and use what it adds over vanilla. For mod
//! authors: `docs/modding.md`.
//!
//! - `ModernEngine.Version`: the API's version (1).
//! - `ModernEngine.registerFont(path, options = null)`: a font file in the game's files
//!   (a mod's zip included) for engine-drawn text the bitmap fonts can't draw:
//!   - `For`: the vanilla fonts it stands in for, a name or an array of them: bitmap
//!     fonts ([`BITMAP_FONTS`]; each also names its typeface on the UI's page) or the
//!     page's font families ([`UI_FAMILIES`]); or `"*"` (the default) for all;
//!   - `Scripts`: the Unicode scripts it's for, by name (`"Han"`, `"Cyrillic"`), a name
//!     or an array; every script by default;
//!   - `Priority`: higher first (0 by default); at equal priority, a later registration
//!     comes first, so mods override in load order.
//!
//!   It returns the font's family name and throws on a file it can't read, a file that
//!   isn't a font, or an unknown `For` or `Scripts` value. Registrations are kept for the
//!   hosts (`Engine::font_registrations`): the renderer's text drawn after one uses it,
//!   and the UI page gets an `@font-face` per family it stands in for, for the code
//!   points vanilla's fonts weren't made for ([`FontRegistration::ui_unicode_range`]).

use std::rc::Rc;

use squirrel::{Retained, SQVM, SqRef, SqTable, Variadic};
use unicode_script::Script;

use crate::font_file::{family_name, is_font};
use crate::source::ScriptSource;
use crate::state::SharedState;

/// The API's version (`ModernEngine.Version`).
pub const VERSION: i64 = 1;

/// The bitmap fonts vanilla draws engine text with (`gfx/fonts/<name>.fnt`).
pub const BITMAP_FONTS: [&str; 3] = ["cinzel_20", "cinzel_bold_20", "cinzel_bold_100"];

/// The font families vanilla's UI page defines (`ui/fonts.css`), each one typeface.
pub const UI_FAMILIES: [&str; 6] = [
	"Arsenal-Regular",
	"Arsenal-Italic",
	"FreeUniversal-Regular",
	"FreeUniversal-Italic",
	"Cinzel-Regular",
	"Cinzel-Bold",
];

/// The page's family for a bitmap font's typeface (both are Cinzel).
fn ui_family_of(bitmap_font: &str) -> Option<&'static str> {
	match bitmap_font {
		"cinzel_20" => Some("Cinzel-Regular"),
		"cinzel_bold_20" | "cinzel_bold_100" => Some("Cinzel-Bold"),
		_ => None,
	}
}

/// A font a mod registered.
#[derive(Clone, Debug, PartialEq)]
pub struct FontRegistration {
	/// Its path in the game's files.
	pub path: String,
	pub data: Vec<u8>,
	pub family: String,
	/// The fonts it stands in for, bitmap fonts and the page's families (`None`: all).
	pub stands_in_for: Option<Vec<String>>,
	/// The Unicode scripts it's for, by name (`None`: all).
	pub scripts: Option<Vec<String>>,
	pub priority: i64,
}

impl FontRegistration {
	/// The UI page's font families it stands in for: those `For` names, and those of the
	/// bitmap fonts it names (all of them without a `For`).
	pub fn ui_families(&self) -> Vec<&'static str> {
		let Some(names) = &self.stands_in_for else {
			return UI_FAMILIES.to_vec();
		};
		UI_FAMILIES
			.into_iter()
			.filter(|family| names.iter().any(|n| n == family || ui_family_of(n) == Some(family)))
			.collect()
	}

	/// The code points it's for on the UI page, as a CSS `unicode-range`: those outside
	/// Windows-1252 (vanilla's fonts keep their text), of its scripts if it names some.
	pub fn ui_unicode_range(&self) -> String {
		let scripts: Option<Vec<Script>> = self
			.scripts
			.as_ref()
			.map(|names| names.iter().filter_map(|n| Script::from_full_name(n)).collect());
		crate::unicode_ranges::css_unicode_range(&crate::unicode_ranges::code_point_ranges(scripts.as_deref()))
	}
}

/// A name, or each name of an array (`None` for null).
fn names(value: Option<SqRef<'_>>, what: &str) -> Result<Option<Vec<String>>, String> {
	match value {
		None | Some(SqRef::Null) => Ok(None),
		Some(SqRef::String(name)) => Ok(Some(vec![name])),
		Some(SqRef::Array(array)) => (0..array.len())
			.map(|i| match array.get_ref(i) {
				Some(SqRef::String(name)) => Ok(name),
				_ => Err(format!("{what}: names are strings")),
			})
			.collect::<Result<Vec<_>, _>>()
			.map(Some),
		Some(_) => Err(format!("{what}: a name or an array of names")),
	}
}

/// Check and keep a registration from `registerFont`'s arguments.
fn register<S: ScriptSource>(vm: &SQVM, state: &SharedState<S>, path: &str, options: Option<&Retained>) -> Result<String, String> {
	let options = options.and_then(|o| o.0.as_table(vm));
	let member = |key: &str| options.as_ref().and_then(|t| t.get_ref(key));
	let stands_in_for = names(member("For"), "For")?.filter(|f| !f.iter().any(|f| f == "*"));
	let known = |f: &str| BITMAP_FONTS.contains(&f) || UI_FAMILIES.contains(&f);
	if let Some(unknown) = stands_in_for.iter().flatten().find(|f| !known(f)) {
		return Err(format!(
			"registerFont({path:?}): no font {unknown:?} (a bitmap font, one of {BITMAP_FONTS:?}; a UI font family, one of {UI_FAMILIES:?}; or \"*\")"
		));
	}
	let scripts = names(member("Scripts"), "Scripts")?;
	if let Some(unknown) = scripts.iter().flatten().find(|s| Script::from_full_name(s).is_none()) {
		return Err(format!("registerFont({path:?}): no Unicode script {unknown:?} (e.g. \"Han\", \"Cyrillic\")"));
	}
	let priority = match member("Priority") {
		None | Some(SqRef::Null) => 0,
		Some(SqRef::Integer(p)) => p,
		Some(_) => return Err(format!("registerFont({path:?}): Priority is an integer")),
	};
	let data = state
		.borrow()
		.source
		.read_file(path)
		.ok_or_else(|| format!("registerFont({path:?}): no such file"))?;
	if !is_font(&data) {
		return Err(format!("registerFont({path:?}): not a TrueType, OpenType or collection font"));
	}
	let family = family_name(&data).ok_or_else(|| format!("registerFont({path:?}): the font has no family name"))?;
	state.borrow_mut().fonts.push(Rc::new(FontRegistration {
		path: path.to_string(),
		data,
		family: family.clone(),
		stands_in_for,
		scripts,
		priority,
	}));
	Ok(family)
}

/// Build the `ModernEngine` table and slot it into the root.
pub(crate) fn install<S: ScriptSource + 'static>(vm: &mut SQVM, state: &SharedState<S>) {
	let register_font = {
		let state = state.clone();
		vm.make_closure(move |vm: &SQVM, path: &str, Variadic(options): Variadic<Retained>| -> Result<String, String> {
			register(vm, &state, path, options.first())
		})
	};
	let mut table = SqTable::new(vm);
	table.set("Version", VERSION);
	table.set("registerFont", register_font);
	SqTable::root(vm).set("ModernEngine", table);
}
