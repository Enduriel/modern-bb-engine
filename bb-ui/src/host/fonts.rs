//! Text laid out in whole pixels, as Coherent lays it out:
//! each glyph advances by a whole number of pixels, its hinted width, so a line is the
//! sum of those. Blink positions glyphs at fractions of a pixel unless fontconfig asks
//! for full hinting, and desktops ask for slight hinting: lines came out up to 0.9%
//! narrower than the original's and wrapped differently (the intro event's text).
//!
//! Glyphs are antialiased in grey, as Coherent's are: a desktop that asks
//! for subpixel (LCD) antialiasing gave every glyph coloured fringes, about twice the
//! original's faint edge pixels, and text up to 17% wider in ink. Chromium's
//! `--text-contrast`/`--text-gamma` don't change grey glyphs on Linux.
//!
//! The browser process points fontconfig at a file that reads the usual configuration
//! and then asks for full hinting and grey antialiasing for every font. Chromium's child
//! processes inherit that. Nothing else in the process uses fontconfig: the engine draws
//! its own text from bitmap fonts.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

/// The variable fontconfig reads its configuration file from.
const VARIABLE: &str = "FONTCONFIG_FILE";

/// Our files' name prefix, in the temp dir.
const PREFIX: &str = "bb-ui-fonts-";

/// The configuration: the usual one (`usual`, else the system's `fonts.conf`), then full
/// hinting and grey antialiasing for every font.
fn config(usual: Option<&str>) -> String {
	let usual = usual.unwrap_or("fonts.conf");
	let usual = usual
		.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;");
	format!(
		r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <include ignore_missing="yes">{usual}</include>
  <match target="font">
    <edit name="hinting" mode="assign"><bool>true</bool></edit>
    <edit name="hintstyle" mode="assign"><const>hintfull</const></edit>
    <edit name="antialias" mode="assign"><bool>true</bool></edit>
    <edit name="rgba" mode="assign"><const>none</const></edit>
    <edit name="lcdfilter" mode="assign"><const>lcdnone</const></edit>
  </match>
</fontconfig>
"#
	)
}

/// Point this process's fontconfig, and that of the processes it starts, at [`config`].
///
/// # Safety
///
/// Changes the environment: no other thread may exist yet.
pub(super) unsafe fn hint_fully() -> Result<(), String> {
	let usual = std::env::var(VARIABLE).ok();
	// Started by a process that already did this (it reads ours).
	if usual.as_deref().is_some_and(|f| f.contains(PREFIX)) {
		return Ok(());
	}
	let text = config(usual.as_deref());
	let mut hash = DefaultHasher::new();
	text.hash(&mut hash);
	let path = std::env::temp_dir().join(format!("{PREFIX}{:016x}.conf", hash.finish()));
	if !path.exists() {
		// Written whole, then moved in place: another game may be reading it.
		let partial = PathBuf::from(format!("{}.{}", path.display(), std::process::id()));
		std::fs::write(&partial, &text).map_err(|e| format!("{}: {e}", partial.display()))?;
		std::fs::rename(&partial, &path).map_err(|e| format!("{}: {e}", path.display()))?;
	}
	// SAFETY: the caller's contract: no other thread reads the environment meanwhile.
	unsafe { std::env::set_var(VARIABLE, &path) };
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::config;

	#[test]
	fn reads_the_usual_configuration_first() {
		let system = config(None);
		assert!(system.contains(r#"<include ignore_missing="yes">fonts.conf</include>"#));
		assert!(system.find("<include").unwrap() < system.find("hintfull").unwrap());
		assert!(system.find("<include").unwrap() < system.find("<const>none</const>").unwrap());
		let own = config(Some("/home/a&b/<fonts>.conf"));
		assert!(own.contains(
			"<include ignore_missing=\"yes\">/home/a&amp;b/&lt;fonts&gt;.conf</include>"
		));
	}
}
