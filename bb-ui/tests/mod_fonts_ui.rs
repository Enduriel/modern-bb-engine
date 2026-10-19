//! A mod's font on the real UI: a mod registers its font from its preload script
//! (`ModernEngine.registerFont`; bb-render's test mod, a serif Han font for the Cinzel
//! Bold text), and the page gets an `@font-face` for vanilla's `Cinzel-Bold` limited to
//! the Han code points (`bb_ui::mod_fonts`). Simplified Chinese in a vanilla title class
//! is then drawn from the mod's font, while text vanilla's fonts were made for never
//! loads it. Skips without a Steam install.
//!
//! Run: `cargo test -p bb-ui --features cef --test mod_fonts_ui` (`BB_SHOT=<png>` saves
//! the page with the Chinese title on it).
//!
//! A plain `main` (`harness = false`): Chromium re-runs this binary for its child
//! processes, and CEF can only start once per process.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_ui::host::{Echo, HostConfig, UiSession};
use bb_ui::json::Json;

/// Characters the test font has (bb-render's `tests/fixtures/README.md`).
const CHINESE: &str = "白石镇黑森林";
/// The test font's file in the test mod, and its family.
const FONT_URL: &str = "coui://gfx/mod_han_font_test/bb_test_han_serif.otf";

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition { Ok(()) } else { Err(what.to_string()) }
}

fn test_mod() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../bb-render/tests/fixtures/mod_han_font_test.zip")
}

/// Evaluate `expr` on the page, as text.
fn text(session: &UiSession, expr: &str) -> Result<String, String> {
	match session.eval(expr)? {
		Json::Str(s) => Ok(s),
		other => Ok(other.to_text()),
	}
}

/// Run frames until `expr` is true on the page.
fn wait_for(session: &UiSession, expr: &str, what: &str) -> Result<(), String> {
	let deadline = Instant::now() + Duration::from_secs(20);
	loop {
		if matches!(session.eval(expr)?, Json::Bool(true)) {
			return Ok(());
		}
		ensure(Instant::now() < deadline, format!("{what}: never ({expr})"))?;
		session.run_frames(0.1)?;
	}
}

/// The mod's face for `Cinzel-Bold` on the page (`document.fonts`), as JS.
const MOD_FACE: &str = r#"(function () {
	var found = null;
	document.fonts.forEach(function (f) {
		if (f.family.replace(/"/g, '') === 'Cinzel-Bold' && f.unicodeRange !== 'U+0-10FFFF') found = f;
	});
	return found;
})()"#;

fn a_mods_font_draws_chinese_on_the_real_ui(data_dir: &Path) -> Result<(), String> {
	let config = HostConfig { extra_mods: vec![test_mod()], ..HostConfig::default() };
	let session = UiSession::start(data_dir, config, Echo::Off)?;
	session.boot_menu()?;
	session.run_frames(0.5)?;

	// The faces: one, for the family the mod's `For` names (Cinzel Bold), Han only.
	let sheet = text(&session, "(document.getElementById('bb-modern-engine-fonts') || {}).textContent || ''")?;
	ensure(sheet.matches("@font-face").count() == 1, format!("one face expected:\n{sheet:.400}"))?;
	ensure(sheet.contains("font-family: \"Cinzel-Bold\";") && sheet.contains(FONT_URL), format!("the mod's Cinzel Bold face:\n{sheet:.400}"))?;
	ensure(
		matches!(session.eval(&format!("{MOD_FACE} !== null"))?, Json::Bool(true)),
		"the page doesn't have the face (a range Chromium couldn't read?)",
	)?;
	// The main menu's text is all vanilla's fonts': the mod's face is never loaded.
	let status = text(&session, &format!("{MOD_FACE}.status"))?;
	ensure(status == "unloaded", format!("the mod's face is {status} before any Chinese text"))?;

	// Chinese in a vanilla title class (Cinzel Bold), beside the same text in the mod's
	// font by its own name.
	session.eval(&format!(
		r#"(function () {{
	var own = new FontFace('Mod Font Test Own', 'url({FONT_URL})');
	document.fonts.add(own);
	own.load();
	var add = function (id, style) {{
		var span = document.createElement('span');
		span.id = id;
		span.className = 'title-font-very-big font-color-title';
		span.textContent = '{CHINESE}';
		span.setAttribute('style', 'position: absolute; left: 40px; white-space: nowrap; z-index: 100000; ' + style);
		document.body.appendChild(span);
	}};
	add('mod-font-title', 'top: 200px;');
	add('mod-font-own', 'top: 300px; font-family: "Mod Font Test Own";');
	return true;
}})()"#
	))?;
	wait_for(&session, &format!("{MOD_FACE}.status === 'loaded'"), "the mod's face loaded for the Chinese title")?;
	wait_for(&session, "(function () { var ok = false; document.fonts.forEach(function (f) { if (f.family.replace(/\"/g, '') === 'Mod Font Test Own' && f.status === 'loaded') ok = true; }); return ok; })()", "the font by its own name")?;
	session.run_frames(0.5)?;
	let widths = text(
		&session,
		"document.getElementById('mod-font-title').getBoundingClientRect().width + ' ' + document.getElementById('mod-font-own').getBoundingClientRect().width",
	)?;
	let (title, own) = widths.split_once(' ').ok_or(format!("widths: {widths}"))?;
	let (title, own): (f64, f64) = (title.parse().map_err(|_| widths.clone())?, own.parse().map_err(|_| widths.clone())?);
	ensure(own > 0.0 && (title - own).abs() < 0.5, format!("the title is {title} px wide, the mod's font draws it {own} px"))?;
	if let Some(shot) = std::env::var_os("BB_SHOT") {
		session.screenshot(Path::new(&shot))?;
	}
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let name = "a_mods_font_draws_chinese_on_the_real_ui";
	let data_dir = Path::new(STEAM_DATA_DIR);
	if harness::data_archives(data_dir).map_or(true, |a| a.is_empty()) {
		println!("test {name} ... skipped (no install at {})", data_dir.display());
		return ExitCode::SUCCESS;
	}
	match a_mods_font_draws_chinese_on_the_real_ui(data_dir) {
		Ok(()) => {
			println!("test {name} ... ok");
			ExitCode::SUCCESS
		}
		Err(error) => {
			println!("test {name} ... FAILED\n{error}");
			ExitCode::FAILURE
		}
	}
}
