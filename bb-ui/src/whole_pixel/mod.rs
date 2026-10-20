//! Fonts whose glyphs advance by whole pixels where the browser would place them at
//! fractions of one (macOS).
//!
//! Coherent advances each glyph by its width rounded to whole pixels at the font size,
//! and a line is the sum. On Linux Blink does the same once fontconfig asks for full
//! hinting (`host/fonts.rs`). On macOS it can't be asked: Blink turns subpixel
//! positioning on for every font there (`FontPlatformData::CreateSkFont`, no switch,
//! preference or CSS reaches it), and then takes CoreText's fractional advances as
//! they are. Text ran up to 1.5% wider or narrower and wrapped differently.
//!
//! So the font says so itself: [`variant`] makes a font variable along a hidden size
//! axis (`opsz`, which Blink sets to the CSS font size by itself) whose `HVAR` table
//! makes every glyph's advance at each whole size its rounded width there
//! ([`sizes`] has the layout). Outlines and everything else stay as they were. The
//! page gets this variant of every font it loads over `coui://`, on macOS only.
//! Measured in Chromium 154 on macOS 26: every sample of 2 fonts × 18 sizes × 7 lines
//! comes out exactly as the sum of rounded advances.

mod sfnt;
mod sizes;
mod tables;

use sfnt::{Font, u16_at};

/// Whether the page's fonts need [`variant`] on this platform.
pub const WANTED: bool = cfg!(target_os = "macos");

/// The font `data` with whole-pixel advances at whole sizes, or `None` for a font
/// it can't vary: not a single TrueType/CFF font (a collection, WOFF), already
/// variable, or missing its metrics.
pub fn variant(data: &[u8]) -> Option<Vec<u8>> {
	let mut font = Font::read(data)?;
	if font.table(b"fvar").is_some() || font.table(b"HVAR").is_some() {
		return None;
	}
	let units_per_em = u16_at(font.table(b"head")?, 18)?;
	let glyphs = u16_at(font.table(b"maxp")?, 4)? as usize;
	let metrics = u16_at(font.table(b"hhea")?, 34)? as usize;
	let hmtx = font.table(b"hmtx")?;
	if metrics == 0 || units_per_em == 0 {
		return None;
	}
	let advances: Vec<u16> = (0..glyphs)
		.map(|glyph| u16_at(hmtx, glyph.min(metrics - 1) * 4))
		.collect::<Option<_>>()?;

	let hvar = tables::hvar(&advances, units_per_em)?;
	let (name, axis_name) = tables::add_name(font.table(b"name")?, "Size")?;
	font.tables.insert(*b"HVAR", hvar);
	font.tables.insert(*b"fvar", tables::fvar(axis_name));
	font.tables.insert(*b"name", name);
	// A signature no longer holds for the changed font.
	font.tables.remove(b"DSIG");
	Some(font.write())
}

/// The font `data` under other names: family `family`, full name `full` and
/// PostScript name `postscript` (a font registered beside the one it came from).
pub fn renamed(data: &[u8], family: &str, full: &str, postscript: &str) -> Option<Vec<u8>> {
	let mut font = Font::read(data)?;
	let names = [(1, family), (3, full), (4, full), (6, postscript), (16, family)];
	let name = tables::set_names(font.table(b"name")?, &names)?;
	font.tables.insert(*b"name", name);
	font.tables.remove(b"DSIG");
	Some(font.write())
}

