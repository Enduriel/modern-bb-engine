//! Battle Brothers on the modern engine, in a window.
//!
//! ```text
//! cargo run --release -p bb-app --features ui -- [--mods] [--new-campaign] [--seed SEED]
//!     [--data DIR] [--world-size COLSxROWS] [--shot FRAMES OUT.png] [--mute]
//!     [--devtools PORT] [--settings PATH] [--saves DIR] [--at FRAMES CODE]...
//!     [--at-page FRAMES JS]...
//!     [--logs DIR] [--no-steam] [--steam-api PATH] [--fps]
//! bb --version
//! ```
//!
//! Boots the install's game files and plays them: the main menu, the world map or the
//! battle, the mouse and keys going to the game.
//!
//! - The install is the one `--data` names (its data folder, or the game's folder), else
//!   `BB_DATA_DIR`'s, else the folder the player chose before, else the game in Steam's
//!   libraries (see `bb_app::install`). None found: a window says where bb looked and
//!   asks for the game's folder (or, built without `ui`, the message box says so), and
//!   bb starts again with the folder chosen, which it keeps for next time.
//! - Whatever ends the start (a machine that can't run the game, a game folder that
//!   won't load), the player is told in a message box too, not only on stderr and in
//!   the log (`bb_app::failure`).
//! - Built with `--features ui`, the game's real UI is drawn over it and takes the input
//!   it's under, and the game stops at the main menu, where the player starts or loads
//!   a campaign. `--devtools` serves Chrome DevTools for its page on localhost at that
//!   port.
//! - `--new-campaign` skips the menu: it starts a new campaign at once, as the menu's
//!   Start does (the "early access" origin, on map seed `rust-engine-test` unless
//!   `--seed` gives one; `--seed` alone implies it). Built without `ui` there is no menu
//!   to click, so `bb` always does this.
//! - `--mods` mounts the install's mods (every `.zip` in its data folder) over its
//!   archives, as the original loads them: for the scripts, the UI page, the sprites
//!   and the sounds.
//! - `--world-size` makes the campaigns' maps that many tiles across and down instead of
//!   vanilla's 140x170 (vanilla's worldgen needs more than 130x130).
//! - `--shot` saves the frame after that many frames and quits; `--at` runs the Squirrel
//!   `CODE` after that many frames, and `--at-page` the `JS` in the UI page (for scripted
//!   runs; the page's then runs after the frame's Squirrel).
//! - `--settings` reads and keeps the settings in that file instead of the system's
//!   place for them, and `--saves` keeps the saves in that folder instead of the
//!   system's place for them (`battle-brothers-modern/savegames` in the user's data
//!   folder; never the original's).
//! - The game logs to `log.html` in the original's format (and CEF to `cef.log`), in
//!   `battle-brothers-modern/` in the user's data folder, or the folder `--logs` names;
//!   the earlier sessions' logs are kept there in `logs/`, zipped. The terminal gets
//!   the lines too.
//! - The sound plays on the default output device, unless `--mute`.
//! - `--fps` shows the frame rate in the top-left corner (vanilla's own FPS module,
//!   which the original never shows), as `ui.show_fps = true` in the settings does;
//!   Ctrl+Shift+F shows or hides it while playing. It's off by default.
//! - The game's mouse cursor shows as the system's cursor (the compositor moves it, so
//!   moving the mouse over a still picture draws nothing); `ui.software_cursor = true`
//!   in the settings draws it into the frames instead, as the original does (see
//!   `bb_app::game_cursor`).
//! - On a Steam install (see `bb_steam::install` for the rule), achievements and stats
//!   go to the player's Steam account through Steam's library beside `bb`
//!   (`libsteam_api.so`, `steam_api64.dll` or `libsteam_api.dylib`), or the one
//!   `--steam-api` (or `BB_STEAM_API`) names. `--no-steam` (or `BB_NO_STEAM=1`) never
//!   loads it. Any other install never does either, and without Steam the game runs on
//!   with achievements off (said once in the log).
//! - With `BB_STATS=1` the game's total CPU and memory, its CEF processes included, is
//!   logged every 5 seconds.
//! - Profiling the frame (see `parity/perf/frame_breakdown.py`): `BB_FRAME_TIMES=PATH` writes
//!   each frame's time by span (the step's parts, the draw's, the page's, the bridge's calls)
//!   to that file; with `BB_GPU_TIMES=1` too, each GPU submission's time (timestamp
//!   queries; the frame then waits for the GPU). `BB_SCRIPT_PROFILE=REPORT.md` profiles the
//!   scripts. Under a preloaded jemalloc with profiling on, `BB_HEAP_PROFILE_DIR=DIR` dumps a
//!   heap profile every `BB_HEAP_PROFILE_SECS` (10) seconds (builds without `ui`).
//! - `--version` prints the version, the target it was built for and whether it has the
//!   UI (`bb 0.1.0 (x86_64-unknown-linux-gnu, ui)`), and exits. It needs no display, but
//!   a UI build has loaded CEF's library by then: the release bundles' smoke check.
//! - Every environment variable, these and the ones for measuring, comparing and the
//!   tests: `docs/environment.md`.

mod app;
mod window_mode;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bb_app::game::Start;
use bb_app::steam::SteamOptions;
use bb_app::install::Asked;
use bb_app::settings::Settings;
use bb_engine::install::NotFound;
use winit::event_loop::EventLoop;

/// The map seed of a campaign started at once, unless `--seed` gives one.
const DEFAULT_SEED: &str = "rust-engine-test";

/// The options, Steam's, the log folder (`None`: the system has no data folder), and
/// `--data`'s folder. The options' data folder is left for the lookup (`bb_app::install`).
fn options() -> Result<(app::Options, SteamOptions, Option<PathBuf>, Option<PathBuf>), String> {
	let mut steam = SteamOptions::from_env();
	let mut data = None;
	let mut options = app::Options {
		data_dir: PathBuf::new(),
		// Without the page there's no menu to click.
		start: if cfg!(feature = "ui") { Start::Menu } else { Start::NewCampaign(DEFAULT_SEED.to_string()) },
		mods: false,
		shot: None,
		mute: false,
		world_size: None,
		devtools: None,
		settings: None,
		at: Vec::new(),
		at_page: Vec::new(),
		saves: None,
		steam_build: false,
		steam: None,
		cef_log: None,
		fps: false,
	};
	let mut logs = squirrel_bb::platform::logs_dir();
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().ok_or(format!("{arg} needs a value"));
		match arg.as_str() {
			"--data" => data = Some(PathBuf::from(value()?)),
			"--seed" => options.start = Start::NewCampaign(value()?),
			"--new-campaign" => {
				if let Start::Menu = options.start {
					options.start = Start::NewCampaign(DEFAULT_SEED.to_string());
				}
			}
			"--mods" => options.mods = true,
			"--mute" => options.mute = true,
			"--fps" => options.fps = true,
			"--no-steam" => steam.off = true,
			"--steam-api" => steam.library = Some(PathBuf::from(value()?)),
			"--world-size" => {
				let v = value()?;
				let size = v
					.split_once('x')
					.and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)))
					.ok_or(format!("--world-size COLSxROWS, not {v}"))?;
				options.world_size = Some(size);
			}
			"--devtools" if cfg!(feature = "ui") => {
				options.devtools = Some(value()?.parse().map_err(|e| format!("--devtools PORT: {e}"))?);
			}
			"--devtools" => return Err("--devtools needs bb built with --features ui".to_string()),
			"--settings" => options.settings = Some(PathBuf::from(value()?)),
			"--saves" => options.saves = Some(PathBuf::from(value()?)),
			"--logs" => logs = Some(PathBuf::from(value()?)),
			"--at" => {
				let frames = value()?.parse().map_err(|_| "--at FRAMES CODE")?;
				options.at.push((frames, value()?));
			}
			"--at-page" => {
				let frames = value()?.parse().map_err(|_| "--at-page FRAMES JS")?;
				options.at_page.push((frames, value()?));
			}
			"--shot" => {
				let frames = value()?.parse().map_err(|_| "--shot FRAMES OUT.png")?;
				options.shot = Some((frames, PathBuf::from(value()?)));
			}
			_ => return Err(format!("unknown argument {arg}")),
		}
	}
	Ok((options, steam, logs, data))
}

/// How a run ended without an error.
enum Ended {
	/// The game was played (and quit).
	Played,
	/// The player closed the not-found window.
	Closed,
	/// The player chose the game's folder: bb starts again with these arguments more.
	Relaunch(Vec<std::ffi::OsString>),
}

/// The settings `options` name (`--settings`), or the system's.
fn load_settings(options: &app::Options) -> Settings {
	match &options.settings {
		Some(path) => Settings::load(Some(path.clone())),
		None => Settings::load_default(),
	}
}

/// What the not-found window runs with, from the command line.
#[cfg(feature = "ui")]
fn picker_options(options: &app::Options) -> bb_app::install::picker::Options {
	bb_app::install::picker::Options {
		shot: options.shot.clone(),
		at_page: options.at_page.clone(),
		cef_log: options.cef_log.clone(),
	}
}

#[cfg(not(feature = "ui"))]
fn picker_options(_options: &app::Options) {}

fn main() -> ExitCode {
	bb_app::platform::allow_tracing_if_asked();
	// The browser host re-runs this binary for its child processes.
	#[cfg(feature = "ui")]
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	if std::env::args().nth(1).as_deref() == Some("--version") {
		println!("{}", bb_app::version());
		return ExitCode::SUCCESS;
	}
	bb_app::platform::tune_allocator();
	squirrel_bb::startup::mark("main");
	let mut logs = None;
	// The game's data folder, once found (the failure message names it).
	let mut data_dir = None;
	let mut run = || -> Result<Ended, String> {
		let (mut options, steam, log_dir, data) = options()?;
		if let Some(dir) = log_dir {
			match bb_app::logs::Logs::start(&dir) {
				Ok(started) => {
					options.cef_log = Some(started.cef_log());
					logs = Some(started);
				}
				Err(e) => eprintln!("no log file: {e}"),
			}
		}
		bb_app::failure::show_panics(logs.as_ref().and_then(bb_app::logs::Logs::dir).map(Path::to_path_buf));
		let mut settings = load_settings(&options);
		options.data_dir = match bb_app::install::find(data, &settings) {
			Ok(install) => install.data_dir,
			Err(NotFound::Nowhere(looked)) => {
				let picker = picker_options(&options);
				return match bb_app::install::ask(&looked, &mut settings, picker)? {
					Asked::Closed => Ok(Ended::Closed),
					asked => Ok(Ended::Relaunch(bb_app::install::relaunch_arguments(&asked))),
				};
			}
			Err(bad) => return Err(bad.to_string()),
		};
		data_dir = Some(options.data_dir.clone());
		options.steam_build = bb_app::steam::is_steam_install(&options.data_dir);
		// SAFETY: no other thread runs yet (Steam's start sets the app id in the
		// environment): the stats logger, the renderer's and the page's start after it.
		options.steam = unsafe { bb_app::steam::start(&options.data_dir, &steam) };
		// Heap profiles every so often, under a preloaded jemalloc (`BB_HEAP_PROFILE_DIR`).
		bb_app::heap_profile::spawn_periodic_from_env();
		// The game's total CPU and memory, with its CEF processes, every 5 s.
		if std::env::var_os("BB_STATS").is_some_and(|v| v != "0") {
			bb_app::stats::spawn_logger(std::time::Duration::from_secs(5));
		}
		squirrel_bb::startup::mark("options, logs, steam");
		let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
		squirrel_bb::startup::mark("event loop");
		let display = Box::new(event_loop.owned_display_handle());
		let instance = wgpu::Instance::new(bb_render::Gpu::platform_backends(
			wgpu::InstanceDescriptor::new_with_display_handle_from_env(display),
		));
		squirrel_bb::startup::mark("wgpu instance");
		let mut app = app::App::new(options, settings, instance, event_loop.create_proxy());
		event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
		app.error.take().map_or(Ok(Ended::Played), Err)
	};
	let result = run();
	let log_dir = logs.as_ref().and_then(bb_app::logs::Logs::dir).map(Path::to_path_buf);
	// Every failure is told to the player in words (a machine that can't run the game
	// with the requirements), not only in the log and on stderr.
	let message = result.as_ref().err().map(|e| bb_app::failure::message(e, data_dir.as_deref(), log_dir.as_deref()));
	if let Some(logs) = logs {
		logs.close(message.as_deref());
	}
	let fail = |text: &str| {
		eprintln!("\n{text}");
		bb_app::requirements::dialog::show(bb_app::failure::TITLE, text);
		ExitCode::FAILURE
	};
	match result {
		Ok(Ended::Played | Ended::Closed) => ExitCode::SUCCESS,
		Ok(Ended::Relaunch(extra)) => match bb_app::install::relaunch::relaunch(&extra) {
			Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
			Err(e) => fail(&bb_app::failure::message(&e, data_dir.as_deref(), log_dir.as_deref())),
		},
		Err(e) => {
			eprintln!("error: {e}");
			fail(message.as_deref().unwrap_or(&e))
		}
	}
}
