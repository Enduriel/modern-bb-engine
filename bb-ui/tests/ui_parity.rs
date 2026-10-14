//! UI parity with the original. Three checks on one page, in order:
//!   * `matches_the_originals_screens`: our page, driven through the main menu, the
//!     new-campaign page and the Options screen, matches the original's snapshots
//!     (`tests/goldens/`, captured with `probes/ui_goldens.js`) in structure, text,
//!     boxes and the compared styles, within `goldens::tolerance`;
//!   * `matches_the_originals_world`: likewise for a new campaign started as the
//!     probe harness starts it: the intro event, the world screen, the character
//!     screen, the nearest town and a seeded bandit ambush (`probes/world_golden.js`);
//!   * `follows_the_originals_layout_rules`: `probes/layout_rules.js` gets the
//!     original's answers (`tests/goldens/layout_rules.json`) in our page.
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

fn matches_the_originals_screens(session: &UiSession) -> Result<(), String> {
	compare_all(goldens::capture(session)?)
}

/// Runs after the menu screens: it leaves Options and starts a campaign.
fn matches_the_originals_world(session: &UiSession) -> Result<(), String> {
	session.click_when_ready("Cancel", Duration::from_secs(30))?;
	compare_all(goldens::capture_world(session)?)
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
	let path = goldens::default_dir().join("layout_rules.json");
	let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
	let golden = Json::parse(&text)?;

	let mut differences = Vec::new();
	for group in ["fonts", "overflow"] {
		let Some(Json::Object(answers)) = golden.get(group) else {
			return Err(format!("layout_rules.json has no {group}"));
		};
		for (key, expected) in answers {
			let name = format!("{group}.{key}");
			if LAYOUT_EXCEPTIONS.contains(&name.as_str()) {
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

/// One check on the shared page.
type Check = fn(&UiSession) -> Result<(), String>;

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let checks: [(&str, Check); 3] = [
		(
			"matches_the_originals_screens",
			matches_the_originals_screens,
		),
		("matches_the_originals_world", matches_the_originals_world),
		(
			"follows_the_originals_layout_rules",
			follows_the_originals_layout_rules,
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
