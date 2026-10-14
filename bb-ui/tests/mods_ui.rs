//! Mods' JS on the real UI: with the install's mods mounted (every `.zip` in
//! `data/`, the installed corpus), the game's page runs in the Chromium host and the mod
//! frameworks' own JS does what it does in the original: Modern Hooks' handshake loads
//! every mod's JS and CSS and then resumes the scripts' boot (`resumeOnInit`), and MSU's
//! late JS connects MSU's screens (`connectBackend`). The main menu comes up. Skips
//! without a Steam install or without Modern Hooks.
//!
//! Run: `cargo test -p bb-ui --features cef --test mods_ui`
//!
//! A plain `main` (`harness = false`): Chromium re-runs this binary for its child
//! processes, and CEF can only start once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_engine::tactical_player as player;
use bb_ui::host::{HostConfig, UiSession};
use bb_ui::json::Json;

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition { Ok(()) } else { Err(what.to_string()) }
}

/// A Squirrel condition, evaluated in the scripts.
fn holds(session: &UiSession, expr: &str) -> Result<bool, String> {
	Ok(player::eval_i64(session.engine(), &format!("({expr}) ? 1 : 0"))? == Some(1))
}

fn mods_js_runs_on_the_real_ui(data_dir: &Path) -> Result<(), String> {
	let config = HostConfig { mods: true, ..HostConfig::default() };
	let session = UiSession::start(data_dir, config, false)?;
	session.boot_menu()?;
	ensure(holds(&session, "\"Cursor\" in ::getroottable()")?, "root_state's own onInit never ran (no ::Cursor)")?;

	// Modern Hooks' page side finished its handshake.
	let deadline = Instant::now() + Duration::from_secs(20);
	loop {
		if matches!(session.eval("typeof Hooks !== 'undefined' && Hooks.screenInit === true")?, Json::Bool(true)) {
			break;
		}
		ensure(Instant::now() < deadline, "Modern Hooks' page never set Hooks.screenInit")?;
		session.run_frames(0.1)?;
	}
	// Every mod's JS loaded: MSU's, for one.
	ensure(
		matches!(session.eval("typeof MSU !== 'undefined'")?, Json::Bool(true)),
		"MSU's JS isn't on the page",
	)?;
	// MSU's late JS connected its screens from the page.
	let deadline = Instant::now() + Duration::from_secs(10);
	while !holds(&session, "\"MSU\" in ::getroottable() && ::MSU.Popup.m.JSHandle != null")? {
		ensure(Instant::now() < deadline, "MSU's popup never connected (connectBackend)")?;
		session.run_frames(0.1)?;
	}
	// The main menu is up.
	let deadline = Instant::now() + Duration::from_secs(10);
	loop {
		let buttons = session.visible_buttons()?;
		if buttons.iter().any(|b| b == "New Campaign") {
			break;
		}
		ensure(Instant::now() < deadline, format!("no main menu; visible buttons: {buttons:?}"))?;
		session.run_frames(0.1)?;
	}
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let name = "mods_js_runs_on_the_real_ui";
	let data_dir = Path::new(STEAM_DATA_DIR);
	let has_mh = harness::mod_archives(data_dir).is_ok_and(|mods| {
		mods.iter().any(|m| m.file_name().is_some_and(|n| n.to_string_lossy().starts_with("mod_modern_hooks")))
	});
	if !has_mh {
		println!("test {name} ... skipped (no install with Modern Hooks at {})", data_dir.display());
		return ExitCode::SUCCESS;
	}
	match mods_js_runs_on_the_real_ui(data_dir) {
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
