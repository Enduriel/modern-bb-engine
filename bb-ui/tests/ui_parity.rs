//! UI parity with the original. Nine checks on one page, in order:
//!   * `draws_tiled_backgrounds_as_the_original`: boxes whose background image tiles
//!     draw it upside down where the original does (`probes/background_tiles.js`), the
//!     main menu's buttons among them, checked in a 2560x1358 window;
//!   * `matches_the_originals_screens`: our page, driven through the main menu, the
//!     new-campaign page and the Options screen, matches the original's snapshots
//!     (`tests/goldens/`, captured with `probes/ui_goldens.js`) in structure, text,
//!     boxes and the compared styles, within `goldens::tolerance`;
//!   * `matches_the_originals_ui_scale`: the main menu at UI scale 1.5
//!     (`probes/ui_scale.{nut,js}`);
//!   * `matches_the_originals_world`: likewise for a new campaign started as the
//!     probe harness starts it: the intro event, the world screen, the character
//!     screen, the nearest town and a seeded bandit ambush (`probes/world_golden.js`);
//!   * `follows_the_originals_layout_rules`: `probes/layout_rules.js` gets the
//!     original's answers (`tests/goldens/layout_rules.json`) in our page;
//!   * `follows_the_originals_text_rules`, `follows_the_originals_border_rules` and
//!     `follows_the_originals_centring_rules`: likewise for `probes/text_rules.js`
//!     (whole-pixel glyph advances, no kerning), `probes/border_rules.js` (border widths
//!     under border images) and `probes/centring_rules.js` (the half pixel of centring
//!     by `auto` margins, at UI scales 1, 1.25 and 1.5).
//!
//! Skips if there is no Steam install.
//!
//! Run: `cargo test -p bb-ui --features cef --test ui_parity`
//!
//! A plain `main` (`harness = false`), like `menu_ui`: Chromium re-runs this binary for
//! its child processes, and CEF can only start once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use bb_engine::harness::STEAM_DATA_DIR;
use bb_ui::host::{Echo, HostConfig, UiSession, goldens};
use bb_ui::json::Json;
use bb_ui::parity::{Snapshot, compare};

const LAYOUT_PROBE: &str = include_str!("../probes/layout_rules.js");
const TEXT_PROBE: &str = include_str!("../probes/text_rules.js");
const BORDER_PROBE: &str = include_str!("../probes/border_rules.js");
const CENTRING_PROBE: &str = include_str!("../probes/centring_rules.js");
const FAMILIES_PROBE: &str = include_str!("../probes/font_families.js");
const TILES_PROBE: &str = include_str!("../probes/background_tiles.js");

/// A window the main menu's buttons were seen misdrawn in: an odd
/// height, where the menu is centred on a half pixel.
const ODD_HEIGHT_WINDOW: (u32, u32) = (2560, 1358);

/// The answers of `font_families.js` that we don't reproduce: a font the original's
/// Windows doesn't have but the test machine does (Arimo, which MSU names after
/// `monospace`) is used here, where the original falls back to its default.
const FAMILY_EXCEPTIONS: &[&str] = &["families.Arimo"];

/// The answers of `layout_rules.js` that we don't reproduce:
/// - text extents and glyph-box heights, which depend on the font rasterizer (old
///   WebKit also rounds font sizes to whole pixels: `5pt` is 7px there, 6.67px here);
/// - `em` lengths under `body`: old WebKit resolves them against the clamped 8px,
///   Blink against the specified 6.25px. Vanilla CSS has no `em` lengths.
const LAYOUT_EXCEPTIONS: &[&str] = &[
	"fonts.px6",
	"fonts.pct50",
	"fonts.em05",
	"fonts.rem05",
	"fonts.pt5",
	"fonts.bodyText",
	"fonts.bodyEm10",
	"fonts.bodyPct50",
	"fonts.bodyPct200",
	"overflow.nowrapText",
];

/// The answers of `text_rules.js` that we don't reproduce:
/// - two lines whose glyphs' hinted widths add up a pixel apart (FreeType's hinting
///   against the original's rasterizer);
/// - kerning asked for with `font-kerning: normal`: the original ignores it, and our
///   built-in `none` gives way to it (no page asks).
const TEXT_EXCEPTIONS: &[&str] = &["text.free18line", "text.cinzelNew", "text.cinzelAVkerning"];

fn start(data_dir: &Path) -> Result<UiSession, String> {
	let (width, height) = goldens::load(&goldens::default_dir(), goldens::SCREENS[0].0)?.viewport;
	let config = HostConfig {
		width: width as u32,
		height: height as u32,
		..HostConfig::default()
	};
	let session = UiSession::start(data_dir, config, Echo::Off)?;
	session.boot_menu()?;
	Ok(session)
}

fn report(differences: Vec<String>) -> Result<(), String> {
	if differences.is_empty() {
		Ok(())
	} else {
		Err(format!(
			"{} differences from the original:\n{}",
			differences.len(),
			differences.join("\n")
		))
	}
}

/// The differences of each snapshot from its golden.
fn compare_all(snapshots: Vec<(&str, Snapshot)>) -> Result<(), String> {
	let dir = goldens::default_dir();
	let mut differences = Vec::new();
	for (name, ours) in snapshots {
		let golden = goldens::load(&dir, name)?;
		for difference in compare(&golden, &ours, &goldens::tolerance(name)) {
			differences.push(format!("{name}: {difference}"));
		}
	}
	report(differences)
}

/// Ends back on the main menu (it leaves Options).
fn matches_the_originals_screens(session: &UiSession) -> Result<(), String> {
	let compared = compare_all(goldens::capture(session)?);
	session.click_when_ready("Cancel", Duration::from_secs(30))?;
	compared
}

/// From the main menu: the menu at UI scales 1.25 and 1.5 (the page laid out at the
/// screen size divided by the scale, drawn at the screen's resolution).
fn matches_the_originals_ui_scale(session: &UiSession) -> Result<(), String> {
	let mut snapshots = Vec::new();
	for (scale, name) in goldens::UI_SCALE_SCREENS {
		snapshots.push((name, goldens::capture_ui_scale(session, scale)?));
	}
	compare_all(snapshots)
}

/// From the main menu: starts a campaign.
fn matches_the_originals_world(session: &UiSession) -> Result<(), String> {
	compare_all(goldens::capture_world(session)?)
}

/// Tiled backgrounds draw upside down, as in the original (`host/tiles.js`): of the
/// boxes on `probes/background_tiles.js`'s card, those the original drew upside down
/// (`tests/goldens/background_tiles.json`) are marked so and no others, and a marked
/// box paints its picture's rows in reverse. Then the main menu in that window:
/// each button paints the gold edges of its picture (43px tall in a 46px box) where
/// the original does, 5 and 33px below its top (its rows 37 and 9 upside down; drawn
/// as it is, they would be 9 and 37 below).
fn draws_tiled_backgrounds_as_the_original(session: &UiSession) -> Result<(), String> {
	let path = goldens::default_dir().join("background_tiles.json");
	let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
	let golden = Json::parse(&text)?;
	let names = |key: &str| -> Result<Vec<String>, String> {
		golden
			.get(key)
			.and_then(Json::as_array)
			.map(|names| names.iter().filter_map(Json::as_str).map(str::to_string).collect())
			.ok_or_else(|| format!("{} has no {key}", path.display()))
	};
	let mut differences = Vec::new();

	let card = TILES_PROBE.split("BbProbe.run").next().unwrap_or_default();
	session.eval(&format!("(function () {{ {card}\n BBTileCard.show(); return true; }})()"))?;
	session.run_frames(1.0)?;
	let marked = session.eval(
		"Array.prototype.map.call(document.querySelectorAll('#bb-tile-card [data-bb-tiled]'), function (e) { return e.className.replace('bb-tile-', ''); })",
	)?;
	let marked: Vec<&str> = marked.as_array().unwrap_or_default().iter().filter_map(Json::as_str).collect();
	for name in names("upside_down")? {
		if !marked.contains(&name.as_str()) {
			differences.push(format!("card: {name}: drawn as it is, the original draws it upside down"));
		}
	}
	for name in names("as_is")? {
		if marked.contains(&name.as_str()) {
			differences.push(format!("card: {name}: drawn upside down, the original draws it as it is"));
		}
	}
	// The menu button's picture repeated in a 46px box (`menu`) against the picture once
	// (`norepeat`): the same 43 rows, the other way round.
	let frame = session.host().frame().ok_or("nothing painted")?;
	let pixel = |x: usize, y: usize| {
		let at = (y * frame.width as usize + x) * 4;
		frame.bgra.get(at..at + 4).map(|p| [p[0], p[1], p[2]]).unwrap_or_default()
	};
	let reversed = (0..43).all(|row| {
		(100..330).all(|x| {
			let (a, b) = (pixel(x, 100 + row), pixel(x, 210 + 42 - row));
			a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 2)
		})
	});
	if !reversed {
		differences.push("card: menu: its picture's rows aren't drawn in reverse".into());
	}
	session.eval("document.getElementById('bb-tile-card').remove(); document.getElementById('bb-tile-card-style').remove(); true")?;

	let rows = golden
		.get("button_gold_rows")
		.and_then(Json::as_array)
		.map(|rows| rows.iter().filter_map(Json::as_i64).collect::<Vec<_>>())
		.unwrap_or_default();
	let mode = |(width, height): (u32, u32)| {
		session.engine().run_script(
			&format!("local m = ::Settings.getVideoMode(); m.Width = {width}; m.Height = {height}; ::Settings.setVideoMode(m);"),
			"video_mode",
		)
	};
	mode(ODD_HEIGHT_WINDOW)?;
	let buttons = (|| {
		session.run_frames(2.0)?;
		let buttons = session.eval(
			"Array.prototype.map.call(document.querySelectorAll('.main-menu-module .ui-control'), function (e) {
				var r = e.getBoundingClientRect();
				return [e.textContent, Math.round(r.left + r.width / 2), Math.round(r.top), Math.round(r.height), e.hasAttribute('data-bb-tiled')];
			})",
		)?;
		Ok::<_, String>((buttons, session.host().frame().ok_or("nothing painted")?))
	})();
	mode((2560, 1440))?;
	session.run_frames(1.0)?;
	let (buttons, frame) = buttons?;
	let Some(buttons) = buttons.as_array().filter(|b| !b.is_empty()) else {
		return Err("no main menu buttons".into());
	};
	if (frame.width, frame.height) != ODD_HEIGHT_WINDOW {
		return Err(format!("the page is {}x{}, not {ODD_HEIGHT_WINDOW:?}", frame.width, frame.height));
	}
	for button in buttons {
		let field = |i: usize| button.as_array().and_then(|f| f.get(i));
		let label = field(0).and_then(Json::as_str).unwrap_or("?");
		let number = |i: usize| field(i).and_then(Json::as_i64).unwrap_or(0) as usize;
		let (x, top, height) = (number(1), number(2), number(3));
		if field(4) != Some(&Json::Bool(true)) {
			differences.push(format!("menu: {label}: not drawn upside down"));
		}
		// Above and below the label's glyphs, which are gold too.
		let gold: Vec<i64> = (0..height)
			.filter(|row| !(9..=31).contains(row))
			.filter(|row| {
				let at = ((top + row) * frame.width as usize + x) * 4;
				let p = frame.bgra.get(at..at + 3).unwrap_or_default();
				p.len() == 3 && p[2].abs_diff(202) <= 6 && p[1].abs_diff(146) <= 6 && p[0].abs_diff(29) <= 6
			})
			.map(|row| row as i64)
			.collect();
		if gold != rows {
			differences.push(format!("menu: {label}: gold edges {gold:?} below its top, the original's {rows:?}"));
		}
	}
	report(differences)
}

/// Each answer `probe` sent under `groups` against the original's (`tests/goldens/
/// {golden}.json`), but `exceptions`.
fn compare_answers(
	ours: &Json,
	golden: &str,
	groups: &[&str],
	exceptions: &[&str],
) -> Result<(), String> {
	let path = goldens::default_dir().join(format!("{golden}.json"));
	let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
	let golden = Json::parse(&text)?;

	let mut differences = Vec::new();
	for group in groups {
		let Some(Json::Object(answers)) = golden.get(group) else {
			return Err(format!("{} has no {group}", path.display()));
		};
		for (key, expected) in answers {
			let name = format!("{group}.{key}");
			if exceptions.contains(&name.as_str()) {
				continue;
			}
			let actual = ours.get(group).and_then(|answers| answers.get(key));
			if actual != Some(expected) {
				let actual = actual.map_or("nothing".to_string(), Json::to_text);
				differences.push(format!("{name}: {} -> {actual}", expected.to_text()));
			}
		}
	}
	report(differences)
}

/// Runs after the screens: the probe leaves its font samples in the page.
fn follows_the_originals_layout_rules(session: &UiSession) -> Result<(), String> {
	let ours = session.eval(&format!(
		"(function () {{
			var BbProbe = {{ answers: {{}}, send: function (key, json) {{ this.answers[key] = JSON.parse(json); }}, done: function () {{}} }};
			{LAYOUT_PROBE}
			BbProbe.run();
			return BbProbe.answers;
		}})()"
	))?;
	compare_answers(
		&ours,
		"layout_rules",
		&["fonts", "overflow"],
		LAYOUT_EXCEPTIONS,
	)
}

/// The answers of a probe that measures a moment after it starts (its fonts or style
/// sheets arrive in between), as the page's timers run.
fn later_answers(session: &UiSession, probe: &str) -> Result<Json, String> {
	session.eval(&format!(
		"(function () {{
			window.BbProbe = {{ answers: {{}}, finished: false,
				send: function (key, json) {{ this.answers[key] = JSON.parse(json); }},
				done: function () {{ this.finished = true; }} }};
			{probe}
			BbProbe.run();
			return true;
		}})()"
	))?;
	for _ in 0..40 {
		session.run_frames(0.25)?;
		if session.eval("BbProbe.finished")? == Json::Bool(true) {
			return session.eval("BbProbe.answers");
		}
	}
	Err("the probe didn't finish within 10 s".into())
}

fn follows_the_originals_text_rules(session: &UiSession) -> Result<(), String> {
	let ours = later_answers(session, TEXT_PROBE)?;
	compare_answers(&ours, "text_rules", &["text"], TEXT_EXCEPTIONS)
}

fn follows_the_originals_font_families(session: &UiSession) -> Result<(), String> {
	let ours = later_answers(session, FAMILIES_PROBE)?;
	compare_answers(&ours, "font_families", &["families"], FAMILY_EXCEPTIONS)
}

fn follows_the_originals_border_rules(session: &UiSession) -> Result<(), String> {
	let ours = later_answers(session, BORDER_PROBE)?;
	compare_answers(&ours, "border_rules", &["borders"], &[])
}

/// The answers of `centring_rules.js` that we don't reproduce: under a zoom, old WebKit
/// first cuts every zoomed length down to whole screen pixels (a 3px inset is 4 at 1.5,
/// a 91px box 113 at 1.25) and centres in what is left; Chromium keeps the fractions.
/// That would take every length rounded so, which isn't emulated; the half pixel at
/// UI scale 1, and everything else at all three, is.
const CENTRING_EXCEPTIONS: &[&str] = &[
	"centring125.negativeHalf",
	"centring150.odd",
	"centring150.insetHalf",
	"centring150.horizontalWhole",
	"centring150.horizontalInsetHalf",
];

/// At UI scales 1, 1.25 and 1.5, as the original was measured: each set as the Options
/// screen does, then measured once the page has laid out again.
fn follows_the_originals_centring_rules(session: &UiSession) -> Result<(), String> {
	session.eval(&format!(
		"(function () {{
			window.BbProbe = {{ answers: {{}}, finished: false,
				send: function (key, json) {{ this.answers[key] = JSON.parse(json); }},
				done: function () {{ this.finished = true; }} }};
			{CENTRING_PROBE}
			return true;
		}})()"
	))?;
	let measured = (|| {
		for (scale, key) in [(1.0, "centring"), (1.25, "centring125"), (1.5, "centring150")] {
			goldens::set_ui_scale(session, scale)?;
			session.run_frames(1.0)?;
			session.eval(&format!(
				"BbProbe.finished = false; BbProbe.measureCentring({scale}, '{key}', function () {{ BbProbe.finished = true; }}); true"
			))?;
			let mut finished = false;
			for _ in 0..40 {
				session.run_frames(0.25)?;
				if session.eval("BbProbe.finished")? == Json::Bool(true) {
					finished = true;
					break;
				}
			}
			if !finished {
				return Err(format!("the probe didn't finish at UI scale {scale}"));
			}
		}
		session.eval("BbProbe.answers")
	})();
	goldens::set_ui_scale(session, 1.0)?;
	session.run_frames(1.0)?;
	compare_answers(&measured?, "centring_rules", &["centring", "centring125", "centring150"], CENTRING_EXCEPTIONS)
}

/// One check on the shared page.
type Check = fn(&UiSession) -> Result<(), String>;

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let checks: [(&str, Check); 9] = [
		(
			"draws_tiled_backgrounds_as_the_original",
			draws_tiled_backgrounds_as_the_original,
		),
		(
			"matches_the_originals_screens",
			matches_the_originals_screens,
		),
		(
			"matches_the_originals_ui_scale",
			matches_the_originals_ui_scale,
		),
		("matches_the_originals_world", matches_the_originals_world),
		(
			"follows_the_originals_layout_rules",
			follows_the_originals_layout_rules,
		),
		(
			"follows_the_originals_text_rules",
			follows_the_originals_text_rules,
		),
		(
			"follows_the_originals_border_rules",
			follows_the_originals_border_rules,
		),
		(
			"follows_the_originals_font_families",
			follows_the_originals_font_families,
		),
		(
			"follows_the_originals_centring_rules",
			follows_the_originals_centring_rules,
		),
	];
	let data_dir = Path::new(STEAM_DATA_DIR);
	if !data_dir.exists() {
		for (name, _) in checks {
			println!(
				"test {name} ... skipped ({} not present)",
				data_dir.display()
			);
		}
		return ExitCode::SUCCESS;
	}
	let session = match start(data_dir) {
		Ok(session) => session,
		Err(error) => {
			println!("test ui_parity ... FAILED to start\n{error}");
			return ExitCode::FAILURE;
		}
	};
	let mut failed = false;
	for (name, check) in checks {
		match check(&session) {
			Ok(()) => println!("test {name} ... ok"),
			Err(error) => {
				failed = true;
				println!("test {name} ... FAILED\n{error}");
			}
		}
	}
	if failed {
		ExitCode::FAILURE
	} else {
		ExitCode::SUCCESS
	}
}
