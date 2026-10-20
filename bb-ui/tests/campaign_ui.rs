//! End-to-end through the real UI: a new campaign started from the main menu the way
//! a player does it ("New Campaign", pick "A New Company", "Next", "Next", hover the
//! map seed for its tooltip and type it, "Start"), through worldgen and the loading
//! screen, to the origin's intro event and, after "Huzzah!", the running world screen.
//! Then back to the main menu and into a new campaign, twice: the second time round,
//! the page loads none of the pictures again (they stay loaded, as in Coherent's image
//! cache), and the screens taken down leave no handlers behind. Last, the campaign is
//! saved, and double-clicking the save in "Load Campaign" loads it.
//! The game's `ui/` runs in the Chromium host (offscreen, headless) against the real
//! scripts. Skips if there is no Steam install.
//!
//! Run: `cargo test -p bb-ui --features cef --test campaign_ui`
//!
//! A plain `main` (`harness = false`), like `menu_ui`: Chromium re-runs this binary for
//! its child processes, and CEF can only start once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use bb_engine::harness::STEAM_DATA_DIR;
use bb_ui::host::{Echo, HostConfig, UiSession};

/// How long a step may take: a new campaign generates its world first.
const STEP: Duration = Duration::from_secs(120);

/// The map seed.
const SEED: &str = "bbuitest";

/// The map seed's text field (the company name's takes 32 characters).
const SEED_FIELD: &str = ".new-campaign-menu-module input[maxlength=\"10\"]";

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition {
		Ok(())
	} else {
		Err(what.to_string())
	}
}

/// `result`, saying what was being done if it failed.
fn during<T>(what: &str, result: Result<T, String>) -> Result<T, String> {
	result.map_err(|error| format!("{what}: {error}"))
}

fn starts_a_campaign_and_reaches_the_world_screen(data_dir: &Path) -> Result<(), String> {
	let session = UiSession::start(data_dir, HostConfig::default(), Echo::Off)?;
	during("booting the menu", session.boot_menu())?;
	play(&session).map_err(|error| {
		// Enough to tell what happened: the scripts' states and the last frame.
		let shot = std::env::temp_dir().join("campaign_ui-failure.png");
		let saved = match session.screenshot(&shot) {
			Ok(Some(_)) => format!("\nlast frame: {}", shot.display()),
			_ => String::new(),
		};
		let states = session.engine().root_state().state_names();
		format!("{error}\nscript states: {states:?}{saved}")
	})
}

fn play(session: &UiSession) -> Result<(), String> {
	for step in ["New Campaign", "A New Company", "Next", "Next"] {
		during(
			&format!("clicking {step:?}"),
			session.click_when_ready(step, STEP),
		)?;
	}
	// Hovering the seed field shows its tooltip, which the page asks the scripts for.
	during(
		"hovering the seed field",
		session.hover_when_ready(&format!("css:{SEED_FIELD}"), STEP),
	)?;
	during(
		"waiting for the seed's tooltip",
		session.wait_for_text("A map seed is a unique string", Duration::from_secs(10)),
	)?;

	// A fixed map seed, so the run is the same every time; the menu fills the field with
	// a random one.
	during("typing the seed", session.fill_field(SEED_FIELD, SEED))?;
	let typed = session.eval("Screens.MainMenuScreen.mNewCampaignModule.mSeed.getInputText()")?;
	ensure(
		typed.as_str() == Some(SEED),
		format!(
			"the seed field holds {} after typing {SEED:?}",
			typed.to_text()
		),
	)?;
	during(
		"clicking \"Start\"",
		session.click_when_ready("Start", STEP),
	)?;

	// The origin's intro event, over the generated world.
	during(
		"waiting for the intro event",
		session.wait_for_text("Huzzah!", STEP),
	)?;
	during(
		"clicking \"Huzzah!\"",
		session.click_when_ready("Huzzah!", STEP),
	)?;

	// The world screen: the top bar shows the day and the company's crowns.
	during(
		"waiting for the world screen",
		session.wait_for_text("Day 1", STEP),
	)?;
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
	back_to_the_menu_and_into_new_campaigns(session)?;
	double_clicking_a_save_loads_it(session)?;

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

/// Every jQuery handler on the page's attached elements, the document and the window.
const HANDLERS: &str = r#"(function () {
	var count = 0;
	function add(el) {
		var events = jQuery._data(el, 'events');
		for (var type in events || {}) count += events[type].length;
	}
	Array.prototype.forEach.call(document.getElementsByTagName('*'), add);
	add(document);
	add(window);
	return count;
})()"#;

/// Quit to the main menu, as its Quit does, and wait for the menu.
fn back_to_the_menu(session: &UiSession) -> Result<(), String> {
	session
		.engine()
		.run_script("::World.State.exitGame();", "quit to the menu")?;
	let deadline = std::time::Instant::now() + STEP;
	loop {
		session.run_frames(0.2)?;
		let names = session.engine().root_state().state_names();
		if !names.iter().any(|n| n == "WorldState")
			&& session
				.visible_buttons()?
				.iter()
				.any(|b| b == "New Campaign")
		{
			break;
		}
		ensure(
			std::time::Instant::now() < deadline,
			format!("never got back to the main menu: {names:?}"),
		)?;
	}
	session.run_frames(1.0)
}

/// From the main menu, a new campaign to its world screen.
fn new_campaign(session: &UiSession) -> Result<(), String> {
	session.start_campaign(SEED)?;
	during(
		"waiting for the intro event",
		session.wait_for_text("Huzzah!", STEP),
	)?;
	during(
		"clicking \"Huzzah!\"",
		session.click_when_ready("Huzzah!", STEP),
	)?;
	during(
		"waiting for the world screen",
		session.wait_for_text("Day 1", STEP),
	)
}

fn back_to_the_menu_and_into_new_campaigns(session: &UiSession) -> Result<(), String> {
	back_to_the_menu(session)?;
	new_campaign(session)?;
	back_to_the_menu(session)?;
	let handlers = session.eval(HANDLERS)?.as_i64();
	let served = session.host().images_served();
	new_campaign(session)?;
	back_to_the_menu(session)?;
	// At most the loading screen's pictures, which it picks at random, are new.
	let again = session.host().images_served() - served;
	ensure(
		again <= 4,
		format!("the page loaded {again} images again for the same screens"),
	)?;
	let after = session.eval(HANDLERS)?.as_i64();
	ensure(
		after == handlers,
		format!("the page's handlers went from {handlers:?} to {after:?} over a campaign"),
	)
}

/// The save [`double_clicking_a_save_loads_it`] makes.
const SAVE: &str = "bb_probe_double_click";

/// From the main menu: a new campaign, saved (to a fresh folder) and quit; then
/// "Load Campaign" and a double click on the save's entry loads it, as vanilla's list
/// does (`assignListCampaignDoubleClickHandler`, a jQuery `dblclick`).
fn double_clicking_a_save_loads_it(session: &UiSession) -> Result<(), String> {
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("campaign_ui_saves");
	let _ = std::fs::remove_dir_all(&dir);
	session.engine().saves().set_directory(dir.clone());
	let result = save_and_double_click(session, &dir);
	let _ = std::fs::remove_dir_all(&dir);
	result
}

fn save_and_double_click(session: &UiSession, dir: &Path) -> Result<(), String> {
	new_campaign(session)?;
	session
		.engine()
		.run_script(&format!("::World.State.saveCampaign({SAVE:?});"), "save")?;
	ensure(
		std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()),
		format!("no save written to {}", dir.display()),
	)?;
	back_to_the_menu(session)?;
	during(
		"clicking \"Load Campaign\"",
		session.click_when_ready("Load Campaign", STEP),
	)?;
	during(
		"double-clicking the save",
		session.double_click_when_ready(SAVE, STEP),
	)?;
	let deadline = std::time::Instant::now() + STEP;
	loop {
		session.run_frames(0.2)?;
		let names = session.engine().root_state().state_names();
		if names.iter().any(|n| n == "WorldState") && session.shows_text("Day 1")? {
			return Ok(());
		}
		ensure(
			std::time::Instant::now() < deadline,
			format!("the save didn't load: {names:?}, buttons {:?}", session.visible_buttons()?),
		)?;
	}
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
