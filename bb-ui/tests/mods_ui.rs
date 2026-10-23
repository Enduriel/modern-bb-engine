//! Mods' JS on the real UI: with the install's mods mounted (every `.zip` in
//! `data/`, the installed corpus), the game's page runs in the Chromium host and the mod
//! frameworks' own JS does what it does in the original: Modern Hooks' handshake loads
//! every mod's JS and CSS and then resumes the scripts' boot (`resumeOnInit`), and MSU's
//! late JS connects MSU's screens (`connectBackend`). The main menu comes up, and it and
//! MSU's mod settings screen match the original's with the same mods
//! (`tests/goldens/modded_*.json`, `probes/modded_goldens.js`). The FPS display shows
//! on it, though Modern Hooks replaces the root screen. Skips without a Steam
//! install or without mods; fails if they are not the pinned corpus
//! (`bb_engine::mod_corpus`), which the goldens were taken with.
//!
//! Run: `cargo test -p bb-ui --features cef --test mods_ui`
//!
//! A plain `main` (`harness = false`): Chromium re-runs this binary for its child
//! processes, and CEF can only start once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_engine::harness::STEAM_DATA_DIR;
use bb_engine::mod_corpus::CorpusError;
use bb_engine::tactical_player as player;
use bb_ui::host::{Echo, HostConfig, UiSession, goldens};
use bb_ui::json::Json;
use bb_ui::parity::compare;

#[path = "support/gpu_page.rs"]
mod gpu_page;

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition { Ok(()) } else { Err(what.to_string()) }
}

/// A Squirrel condition, evaluated in the scripts.
fn holds(session: &UiSession, expr: &str) -> Result<bool, String> {
	Ok(player::eval_i64(session.engine(), &format!("({expr}) ? 1 : 0"))? == Some(1))
}

fn mods_js_runs_on_the_real_ui(data_dir: &Path) -> Result<(), String> {
	// The goldens' screen.
	let config = HostConfig { mods: true, width: 2560, height: 1440, sink: Some(gpu_page::sink()?), ..HostConfig::default() };
	let session = UiSession::start(data_dir, config, Echo::Off)?;
	// The mods' own storages as the original keeps them for this install (MSU's settings,
	// say), so the mods start from the same data (copies: never written back).
	session.engine().saves().set_directory(copy_mod_storages()?);
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
	// Nested Tooltips' hooks kept what they wrapped: the mod registers its `setup.js`
	// twice, and the copy resets its namespace (Coherent's script order, see
	// `host/script_order.js`).
	if matches!(session.eval("typeof NestedTooltips !== 'undefined'")?, Json::Bool(true)) {
		ensure(
			matches!(
				session.eval("typeof NestedTooltips.Hooks.WorldEventScreen_updateHeader === 'function' && typeof NestedTooltips.Hooks.WorldEventScreen_renderListItem === 'function'")?,
				Json::Bool(true)
			),
			"Nested Tooltips' hooks lost what they wrapped (its second setup.js ran after them)",
		)?;
	}
	// MSU's late JS connected its screens from the page, before the boot to the menu
	// returned: it waits for the page's inserted scripts to run, so a campaign started
	// at once finds MSU's settings page connected.
	ensure(
		holds(&session, "\"MSU\" in ::getroottable() && ::MSU.Popup.m.JSHandle != null && ::MSU.SettingsScreen.m.JSHandle != null")?,
		"MSU's screens weren't connected when the boot to the menu returned (connectBackend, its late JS)",
	)?;
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
	matches_the_originals_modded_screens(&session)?;
	scroll_bars_stay_still(&session)?;
	fps_display_shows(&session)
}

/// Every scroll bar on the page, the mods' (MSU's settings, Stronghold's, …) and those
/// made after the boot included, is left alone while nothing changes: none runs the
/// plugin's 200ms poll (`host/scroll_bars.js`), which would update each five times a
/// second.
fn scroll_bars_stay_still(session: &UiSession) -> Result<(), String> {
	let bars = session.eval(
		"window.__bbUpdates = 0; $('.aciScrollBar').on('update.bbCount', function () { window.__bbUpdates++; }).length",
	)?;
	ensure(bars.as_i64().is_some_and(|n| n >= 10), format!("only {} scroll bars on the modded page", bars.to_text()))?;
	session.run_frames(1.0)?;
	let updates = session.eval("var n = window.__bbUpdates; $('.aciScrollBar').off('update.bbCount'); delete window.__bbUpdates; n")?;
	ensure(
		updates.as_i64() == Some(0),
		format!("{} scroll bars updated {} times in a second with nothing changing", bars.to_text(), updates.to_text()),
	)
}

/// The FPS display with the mods: Modern Hooks registers a second
/// `RootScreen` over the one the game connected, which never gets a container (the
/// first one's stays on the page). The display shows on the page all the same, with the
/// value the game sets through `callFromSQ`, and that call never reaches a module
/// without its label (vanilla's `setFPS` throws then).
fn fps_display_shows(session: &UiSession) -> Result<(), String> {
	session.show_fps_module(true)?;
	session.set_fps("");
	session.set_fps("123");
	session.run_frames(0.2)?;
	let shown = session.eval(
		"(function () {
			var all = document.querySelectorAll('.root-screen .fps-container .value');
			var out = [];
			for (var i = 0; i < all.length; i++) out.push(all[i].offsetParent !== null ? all[i].textContent : '(hidden)');
			return out.join(',');
		})()",
	)?;
	let errors: Vec<String> = session
		.log()
		.into_iter()
		.filter(|line| (line.level == "error" || line.level == "bridge") && line.text.contains("FPS"))
		.map(|line| line.text)
		.collect();
	ensure(errors.is_empty(), format!("the FPS display: page errors: {errors:?}"))?;
	ensure(shown.as_str() == Some("123"), format!("the FPS displays show {}, not \"123\"", shown.to_text()))?;
	session.show_fps_module(false)?;
	ensure(
		session.eval("document.querySelectorAll('.fps-container').length")?.as_i64() == Some(0),
		"the FPS display stayed once hidden",
	)
}

/// A fresh folder with copies of the installed mods' storages from the original's savegames
/// (the `<mod>#<name>.sav` ones; not the saves; those a probe run holds aside, not its
/// own), or an empty one where there are none.
fn copy_mod_storages() -> Result<std::path::PathBuf, String> {
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mods_ui_storages");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
	if let Some(documents) = squirrel_bb::platform::original_documents_dir() {
		bb_engine::probe_aside::copy_user_mod_storages(&documents, &dir).map_err(|e| format!("the mods' storages: {e}"))?;
	}
	Ok(dir)
}

/// What `probes/modded_goldens.js` runs: the visible button `label`'s jQuery click.
fn click_like_the_probe(session: &UiSession, label: &str) -> Result<(), String> {
	let quoted = Json::Str(label.into()).to_text();
	let clicked = session.eval(&format!(
		"(function (label) {{
			var all = document.querySelectorAll('.ui-control');
			for (var i = 0; i < all.length; i++) {{
				if (all[i].offsetParent !== null && all[i].textContent.trim() === label) {{
					$(all[i]).click();
					return true;
				}}
			}}
			return false;
		}})({quoted})"
	))?;
	ensure(clicked == Json::Bool(true), format!("no visible button {label}"))
}

/// The main menu with the mods, and MSU's mod settings screen opened from its "Mod
/// Options" button and left by its Cancel, against the original's with the same mods,
/// taken the same way (`probes/modded_goldens.js`).
fn matches_the_originals_modded_screens(session: &UiSession) -> Result<(), String> {
	let mut differences = Vec::new();
	let mut check = |name: &str| -> Result<(), String> {
		let golden = goldens::load(&goldens::default_dir(), name)?;
		let ours = session.snapshot()?;
		for difference in compare(&golden, &ours, &goldens::tolerance(name)) {
			differences.push(format!("{name}: {difference}"));
		}
		Ok(())
	};
	session.run_frames(3.0)?;
	check("modded_main_menu")?;
	// The cursor where the original's sat while the goldens were taken (the probe's
	// headless display put it at (640, 360); probed: `probes/mouse_events.js`). Dev
	// Console's element inspector box hides on any mouseover. In the original the box
	// comes (late JS) after the cursor's last one, so it shows on the menu, and MSU's
	// screen opening under the cursor hides it. Ours has had no cursor, so moving in
	// now does the same: the box shows on the menu and is hidden by the next screen.
	session.move_to(640, 360)?;
	// As the probe does (a jQuery click): Modern Hooks' error popup lies over the menu's
	// middle at opacity 0 in both engines, and both hit-test it (probed:
	// `probes/hit_opacity.js`), so a mouse click there lands on the popup.
	click_like_the_probe(session, "Mod Options")?;
	session.run_frames(3.0)?;
	check("modded_mod_options")?;
	let buttons = session.visible_buttons()?;
	ensure(
		buttons.iter().any(|b| b == "Cancel"),
		format!(
			"MSU's mod settings didn't open; visible buttons: {buttons:?}; differences so far:\n{}",
			differences.join("\n")
		),
	)?;
	click_like_the_probe(session, "Cancel")?;
	session.run_frames(1.0)?;
	ensure(
		differences.is_empty(),
		format!("{} differences from the original:\n{}", differences.len(), differences.join("\n")),
	)
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// A runner's `--list` (cargo-nextest's), answered before a display starts.
	if let Some(code) = bb_engine::harness::list_case("mods_js_runs_on_the_real_ui") {
		return code;
	}
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	let name = "mods_js_runs_on_the_real_ui";
	let data_dir = Path::new(STEAM_DATA_DIR);
	match bb_engine::mod_corpus::installed(data_dir) {
		Ok(_) => {}
		Err(CorpusError::Absent(why)) => return bb_engine::harness::skip_case(name, why),
		Err(e) => {
			println!("test {name} ... FAILED\n{e}");
			return ExitCode::FAILURE;
		}
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
