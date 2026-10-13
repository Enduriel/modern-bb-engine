//! End-to-end through the real UI: a new campaign started from the main menu the way
//! a player does it ("New Campaign", pick "A New Company", "Next", "Next", "Start"),
//! through worldgen and the loading screen, to the origin's intro event and, after
//! "Huzzah!", the running world screen. The game's `ui/` runs in the Chromium host
//! (offscreen, headless) against the real scripts. Skips if there is no Steam install.
//!
//! Run: `cargo test -p bb-ui --features cef --test campaign_ui`
//!
//! A plain `main` (`harness = false`), like `menu_ui`: Chromium re-runs this binary for
//! its child processes, and CEF can only start once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use bb_engine::harness::STEAM_DATA_DIR;
use bb_ui::host::{HostConfig, UiSession};

/// How long a step may take: a new campaign generates its world first.
const STEP: Duration = Duration::from_secs(120);

/// The map seed (the field takes up to 10 characters).
const SEED: &str = "bbuitest";

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition {
		Ok(())
	} else {
		Err(what.to_string())
	}
}

fn starts_a_campaign_and_reaches_the_world_screen(data_dir: &Path) -> Result<(), String> {
	let session = UiSession::start(data_dir, HostConfig::default(), false)?;
	session.boot_menu()?;

	for step in ["New Campaign", "A New Company", "Next", "Next"] {
		session.click_when_ready(step, STEP)?;
	}
	// A fixed map seed, so the run is the same every time. The menu fills the field with
	// a random one; set it through the field itself, standing in for typing (no keyboard
	// input yet).
	session.eval(&format!(
		"Screens.MainMenuScreen.mNewCampaignModule.mSeed.setInputText({:?})",
		SEED
	))?;
	session.click_when_ready("Start", STEP)?;

	// The origin's intro event, over the generated world.
	session.wait_for_text("Huzzah!", STEP)?;
	session.click_when_ready("Huzzah!", STEP)?;

	// The world screen: the top bar shows the day and the company's crowns.
	session.wait_for_text("Day 1", STEP)?;
	ensure(
		session.shows_text("2,900")?,
		"the top bar doesn't show 2,900 crowns",
	)?;
	ensure(
		session
			.engine()
			.root_state()
			.state_names()
			.contains(&"WorldState".to_string()),
		"no WorldState after the campaign started",
	)?;

	let problems: Vec<String> = session
		.log()
		.into_iter()
		.filter(|line| line.level == "error" || line.level == "bridge")
		.map(|line| format!("[{}] {}", line.level, line.text))
		.collect();
	ensure(
		problems.is_empty(),
		format!("page or bridge errors:\n{}", problems.join("\n")),
	)
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let name = "starts_a_campaign_and_reaches_the_world_screen";
	let data_dir = Path::new(STEAM_DATA_DIR);
	if !data_dir.exists() {
		println!(
			"test {name} ... skipped ({} not present)",
			data_dir.display()
		);
		return ExitCode::SUCCESS;
	}
	match starts_a_campaign_and_reaches_the_world_screen(data_dir) {
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
