//! UI parity with the original. Seven checks on one page, in order:
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
use bb_ui::host::{HostConfig, UiSession, goldens};
use bb_ui::json::Json;
use bb_ui::parity::{Snapshot, compare};

const LAYOUT_PROBE: &str = include_str!("../probes/layout_rules.js");
const TEXT_PROBE: &str = include_str!("../probes/text_rules.js");
const BORDER_PROBE: &str = include_str!("../probes/border_rules.js");
const CENTRING_PROBE: &str = include_str!("../probes/centring_rules.js");

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
	let session = UiSession::start(data_dir, config, false)?;
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
	let checks: [(&str, Check); 7] = [
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
