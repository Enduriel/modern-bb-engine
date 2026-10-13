//! End-to-end through the real UI: the game's `ui/` runs in the Chromium host
//! (offscreen, headless) against the real scripts, boots to the main menu, and a real
//! mouse click on "New Campaign" opens the new-campaign menu, whose origin list the
//! mouse wheel scrolls and which "Cancel" closes; Options round-trips its settings;
//! calls between scripts and page run inline both ways, as in the original. Skips if
//! there is no Steam install.
//!
//! Run: `cargo test -p bb-ui --features cef --test menu_ui`
//!
//! A plain `main` (`harness = false`): Chromium re-runs this binary for its child
//! processes, which must go straight to [`run_subprocess`](bb_ui::host::run_subprocess),
//! and CEF can only start once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_engine::harness::STEAM_DATA_DIR;
use bb_ui::host::{HostConfig, UiSession};

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition {
		Ok(())
	} else {
		Err(what.to_string())
	}
}

/// Run the game until a button labelled `label` is visible and the page has stopped
/// animating, as a player would wait before clicking. Fails after `timeout`.
fn wait_for_button(session: &UiSession, label: &str, timeout: Duration) -> Result<(), String> {
	let deadline = Instant::now() + timeout;
	loop {
		let buttons = session.visible_buttons()?;
		if buttons.iter().any(|b| b == label) && !session.animating()? {
			return Ok(());
		}
		if Instant::now() > deadline {
			return Err(format!(
				"no {label:?} button appeared; visible: {buttons:?}"
			));
		}
		session.run_frames(0.1)?;
	}
}

fn boots_to_the_main_menu_and_opens_new_campaign(data_dir: &Path) -> Result<(), String> {
	let session = UiSession::start(data_dir, HostConfig::default(), false)?;
	let screens = session.protocol().screens();
	ensure(
		screens.len() == 22,
		format!("the page registered {} screens: {screens:?}", screens.len()),
	)?;

	session.boot_menu()?;
	wait_for_button(&session, "New Campaign", Duration::from_secs(10))?;
	let buttons = session.visible_buttons()?;
	for label in [
		"New Campaign",
		"Load Campaign",
		"Scenarios",
		"Options",
		"Credits",
		"Quit",
	] {
		ensure(
			buttons.iter().any(|b| b == label),
			format!("main menu lacks {label:?}: {buttons:?}"),
		)?;
	}

	// What the scripts sent (`setVersion`, `setMOTD`) is on the page.
	let page = session.eval(
		r#"({ version: document.querySelector('.main-menu-screen .version').textContent,
		      motd: document.querySelector('.main-menu-screen .motd').textContent })"#,
	)?;
	let version = page.get("version").and_then(|v| v.as_str()).unwrap_or("");
	ensure(
		version.starts_with("1."),
		format!("version text {version:?}"),
	)?;
	let motd = page.get("motd").and_then(|v| v.as_str()).unwrap_or("");
	ensure(
		motd.starts_with("Battle Brothers is a challenging game"),
		format!("motd {motd:?}"),
	)?;

	// A real click goes page → scripts → page: the menu state opens the new-campaign
	// module, with the origins the scenario manager listed.
	session.click_button("New Campaign")?;
	wait_for_button(&session, "Next", Duration::from_secs(10))?;
	let origins = session.eval(
		r#"document.querySelector('.main-menu-screen').textContent.indexOf('A New Company') >= 0"#,
	)?;
	ensure(
		origins.to_text() == "true",
		"the origin list is missing 'A New Company'",
	)?;

	// The wheel scrolls the origin list (aciScrollBar reads jquery.mousewheel's
	// `wheelDelta / 120`): two notches down, then back up to the top.
	let list = ".new-campaign-menu-module .l-list-container .aciSb_cnt";
	let scroll_top = || {
		session
			.eval(&format!("document.querySelector({list:?}).scrollTop"))
			.map(|top| top.as_i64().unwrap_or(-1))
	};
	session.wheel_over(list, -2)?;
	let down = scroll_top()?;
	ensure(down > 0, format!("two notches down left the origin list at {down}"))?;
	session.wheel_over(list, 2)?;
	let up = scroll_top()?;
	ensure(up == 0, format!("two notches back up left the origin list at {up}"))?;

	session.click_button("Cancel")?;
	wait_for_button(&session, "New Campaign", Duration::from_secs(10))?;

	// Options: the scripts fill it from the display modes and settings, and "Ok"
	// applies every tab's settings back through them.
	session.click_button("Options")?;
	wait_for_button(&session, "1920 x 1080", Duration::from_secs(10))?;
	session.click_button("Ok")?;
	wait_for_button(&session, "New Campaign", Duration::from_secs(10))?;

	// Calls run inline both ways, as in the original: an `asyncCall` returns after the
	// page ran it, including the page's own call back into the scripts.
	session.eval(
		r#"Screens.MainMenuScreen.bbEcho = function (x) { SQ.call(this.mSQHandle, 'bbEchoBack', x + 1); }"#,
	)?;
	session.engine().run_script(
		r#"
		local screen = ::RootState.get("MainMenuState").m.MainMenuScreen;
		screen.bbEchoBack <- function (_x) { ::__bbEchoed <- _x; };
		::__bbEchoed <- null;
		screen.m.JSHandle.asyncCall("bbEcho", 41);
		if (::__bbEchoed != 42) throw "the page's call back arrived after asyncCall returned";
		screen.m.JSHandle.call("setMOTD", "inline MOTD");
		"#,
		"inline_calls",
	)?;
	let motd = session.eval("document.querySelector('.main-menu-screen .motd').textContent")?;
	ensure(
		motd.as_str() == Some("inline MOTD"),
		format!("after JSHandle.call the MOTD is {}", motd.to_text()),
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
	let name = "boots_to_the_main_menu_and_opens_new_campaign";
	let data_dir = Path::new(STEAM_DATA_DIR);
	if !data_dir.exists() {
		println!(
			"test {name} ... skipped ({} not present)",
			data_dir.display()
		);
		return ExitCode::SUCCESS;
	}
	match boots_to_the_main_menu_and_opens_new_campaign(data_dir) {
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
