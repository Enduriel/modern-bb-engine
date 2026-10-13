//! Boot the REAL game to its main menu with the real UI: the game's `ui/main.html`
//! runs in Chromium (offscreen, headless), talking to the game scripts through the
//! Coherent-compatible bridge. Nothing is shown on screen; the result is a screenshot
//! and a summary of the page.
//!
//! With `--click STEP` (repeatable) it then acts, as a player would. A step is a click
//! once the target shows up: the visible button with that label, else any element with
//! that text, or with `css:SELECTOR` the first visible match. `hover:TARGET` moves the
//! mouse over a target instead, `key:NAME` presses a key (`Enter`, `Escape`, … see
//! `Key`), `type:TEXT` types into the focused field, and `sq:CODE` runs Squirrel in the
//! game (e.g. `sq:::World.State.setPause(false)`). It
//! reports and screenshots the result of each step (`menu-1.png`, …). `--seed SEED`
//! types the new-campaign map seed into its field just before a "Start" click.
//! `--after SECONDS` keeps the game running after the last step (e.g. while a new
//! campaign generates), then reports again (`menu-after.png`).
//!
//! `--eval CODE` (repeatable) prints the value of `CODE` evaluated in the page after
//! the clicks. `--devtools PORT` serves Chrome DevTools for the page on localhost
//! (combine with a long `--seconds` to inspect it).
//!
//! Run: `cargo run -p bb-ui --features cef --example boot_menu_ui -- [data-dir]
//!       [--seconds N] [--out menu.png] [--size 1920x1080] [--click "New Campaign"]
//!       [--after 10] [--seed bbuitest] [--eval "document.title"] [--devtools 9222]`

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_engine::harness::STEAM_DATA_DIR;
use bb_ui::host::{HostConfig, Key, UiSession};

/// The new-campaign map seed's text field (the company name's takes 32 characters).
const SEED_FIELD: &str = ".new-campaign-menu-module input[maxlength=\"10\"]";

/// What the page looks like, as JSON (evaluated in the page).
const SUMMARY_JS: &str = r#"(function () {
	var menu = document.querySelector('.main-menu-screen');
	return {
		screens: Object.keys(Screens).length,
		mainMenuShown: !!menu && menu.classList.contains('display-block'),
		version: (document.querySelector('.main-menu-screen .version') || {}).textContent || null,
		motd: ((document.querySelector('.main-menu-screen .motd') || {}).textContent || '').slice(0, 60),
		fonts: document.fonts.status
	};
})()"#;

struct Options {
	data_dir: PathBuf,
	seconds: f64,
	out: PathBuf,
	width: u32,
	height: u32,
	clicks: Vec<String>,
	evals: Vec<String>,
	devtools: Option<u16>,
	bench: u32,
	after: f64,
	seed: Option<String>,
}

/// Time `n` script → page calls, and `n` that call back into the script (each one a
/// blocking round trip to the renderer, nested for the call back).
fn bench(session: &UiSession, n: u32) -> Result<(), String> {
	session.eval(
		r#"Screens.MainMenuScreen.bbNop = function () {};
		Screens.MainMenuScreen.bbEcho = function (x) { SQ.call(this.mSQHandle, 'bbEchoBack', x); };"#,
	)?;
	session.engine().run_script(
		r#"::__bbScreen <- ::RootState.get("MainMenuState").m.MainMenuScreen;
		::__bbScreen.bbEchoBack <- function (_x) {};"#,
		"bench_setup",
	)?;
	for (label, method) in [("asyncCall", "bbNop"), ("asyncCall + call back", "bbEcho")] {
		let start = Instant::now();
		session.engine().run_script(
			&format!(
				"for (local i = 0; i < {n}; i++) ::__bbScreen.m.JSHandle.asyncCall(\"{method}\", i);"
			),
			"bench",
		)?;
		let per_call = start.elapsed() / n;
		println!("bench: {label}: {per_call:?} per call ({n} calls)");
	}
	Ok(())
}

fn parse_options() -> Result<Options, String> {
	let mut options = Options {
		data_dir: PathBuf::from(STEAM_DATA_DIR),
		seconds: 3.0,
		out: PathBuf::from("boot_menu_ui.png"),
		width: 1920,
		height: 1080,
		clicks: Vec::new(),
		evals: Vec::new(),
		devtools: None,
		bench: 0,
		after: 0.0,
		seed: None,
	};
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().ok_or(format!("{arg} needs a value"));
		match arg.as_str() {
			"--seconds" => {
				options.seconds = value()?.parse().map_err(|e| format!("--seconds: {e}"))?
			}
			"--out" => options.out = PathBuf::from(value()?),
			"--click" => options.clicks.push(value()?),
			"--eval" => options.evals.push(value()?),
			"--bench" => options.bench = value()?.parse().map_err(|e| format!("--bench: {e}"))?,
			"--after" => options.after = value()?.parse().map_err(|e| format!("--after: {e}"))?,
			"--seed" => options.seed = Some(value()?),
			"--devtools" => {
				options.devtools = Some(value()?.parse().map_err(|e| format!("--devtools: {e}"))?)
			}
			"--size" => {
				let size = value()?;
				let (w, h) = size.split_once('x').ok_or("--size is WxH")?;
				options.width = w.parse().map_err(|e| format!("--size: {e}"))?;
				options.height = h.parse().map_err(|e| format!("--size: {e}"))?;
			}
			_ if !arg.starts_with("--") => options.data_dir = PathBuf::from(arg),
			_ => return Err(format!("unknown option {arg}")),
		}
	}
	Ok(options)
}

/// Print the page summary and visible buttons, and save a screenshot to `out`.
fn report(session: &UiSession, out: &Path) -> Result<(), String> {
	match session.eval(SUMMARY_JS) {
		Ok(summary) => println!("page: {}", summary.to_text()),
		Err(error) => println!("page summary failed: {error}"),
	}
	println!("buttons: {:?}", session.visible_buttons()?);
	let stats = session.protocol().stats();
	println!(
		"calls into the page so far: {} ({:?} in all)",
		stats.calls, stats.time
	);
	match session.screenshot(out)? {
		Some((w, h)) => println!("screenshot: {} ({w}x{h})", out.display()),
		None => println!("no frame was painted"),
	}
	Ok(())
}

fn run(options: Options) -> Result<(), String> {
	println!("data: {}", options.data_dir.display());
	let started = Instant::now();
	let config = HostConfig {
		width: options.width,
		height: options.height,
		devtools_port: options.devtools,
		..HostConfig::default()
	};
	if let Some(port) = options.devtools {
		println!("DevTools: http://127.0.0.1:{port} (or chrome://inspect)");
	}
	let session = UiSession::start(&options.data_dir, config, true)?;
	println!(
		"page up, {} screens registered, after {:?}",
		session.protocol().screens().len(),
		started.elapsed()
	);

	session.boot_menu()?;
	println!(
		"booted root_state after {:?}; states: {:?}",
		started.elapsed(),
		session.engine().root_state().state_names()
	);

	session.run_frames(options.seconds)?;
	report(&session, &options.out)?;
	if options.bench > 0 {
		bench(&session, options.bench)?;
	}

	let stem = options
		.out
		.file_stem()
		.map_or("menu".into(), |s| s.to_string_lossy().into_owned());
	for (i, step) in options.clicks.iter().enumerate() {
		println!("step {step:?}");
		if let (Some(seed), "Start") = (&options.seed, step.as_str()) {
			session.fill_field(SEED_FIELD, seed)?;
		}
		if let Some(name) = step.strip_prefix("key:") {
			session.press_key(Key::from_name(name).ok_or(format!("no key called {name:?}"))?)?;
		} else if let Some(text) = step.strip_prefix("type:") {
			session.type_text(text)?;
		} else if let Some(target) = step.strip_prefix("hover:") {
			session.hover_when_ready(target, Duration::from_secs(120))?;
		} else if let Some(code) = step.strip_prefix("sq:") {
			session.engine().run_script(code, "step")?;
		} else {
			// Generous: a new campaign takes a while to generate.
			session.click_when_ready(step, Duration::from_secs(120))?;
		}
		session.run_frames(1.5)?;
		report(
			&session,
			&options.out.with_file_name(format!("{stem}-{}.png", i + 1)),
		)?;
	}
	if options.after > 0.0 {
		session.run_frames(options.after)?;
		report(
			&session,
			&options.out.with_file_name(format!("{stem}-after.png")),
		)?;
	}
	for code in &options.evals {
		match session.eval(code) {
			Ok(value) => println!("eval: {}", value.to_text()),
			Err(error) => println!("eval failed: {error}"),
		}
	}

	drop(session);
	println!("shut down after {:?}", started.elapsed());
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	match parse_options().and_then(run) {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("error: {error}");
			ExitCode::FAILURE
		}
	}
}
