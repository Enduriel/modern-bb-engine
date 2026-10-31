//! Battle Brothers on the modern engine, in a window.
//!
//! ```text
//! cargo run --release -p bb-app --features ui -- [--no-mods] [--mods-dir DIR] [--new-campaign] [--seed SEED]
//!     [--data DIR] [--world-size COLSxROWS] [--shot FRAMES OUT.png] [--mute]
//!     [--devtools PORT] [--settings PATH] [--saves DIR] [--at FRAMES CODE]...
//!     [--at-page FRAMES JS]...
//!     [--logs DIR] [--no-steam] [--steam-api PATH] [--fps] [--memory-snapshot-v8]
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
//!   bb starts again with the folder chosen, which it keeps for next time (if it can't,
//!   it passes the folder to itself as `--chosen-data DIR`, the folder chosen before).
//! - Whatever ends the start (a machine that can't run the game, a game folder that
//!   won't load), the player is told in a message box too, not only on stderr and in
//!   the log (`bb_app::failure`). Except the command line's errors (an unknown argument,
//!   a missing or bad value, and `--data` or `BB_DATA_DIR` naming a folder without the
//!   game): a player who double-clicks bb passes neither, so those go to stderr (and the
//!   log, once it's open) only.
//! - Built with `--features ui`, the game's real UI is drawn over it and takes the input
//!   it's under, and the game stops at the main menu, where the player starts or loads
//!   a campaign. `--devtools` serves Chrome DevTools for its page on localhost at that
//!   port.
//! - `--new-campaign` skips the menu: it starts a new campaign at once, as the menu's
//!   Start does (the "early access" origin, on map seed `rust-engine-test` unless
//!   `--seed` gives one; `--seed` alone implies it). Built without `ui` there is no menu
//!   to click, so `bb` always does this.
//! - The install's mods (every `.zip` in its data folder) are mounted over its archives,
//!   as the original loads them: for the scripts, the UI page, the sprites and the
//!   sounds. `--no-mods` (or `BB_NO_MODS=1`) plays vanilla; `--mods` is the default
//!   (it was once needed, and still turns them on after a `--no-mods`: the last wins).
//!   `--mods-dir DIR` mounts the `.zip`s in `DIR` instead of the install's (and turns
//!   the mods on): for tests and comparisons, e.g. the reference mod corpus's
//!   (`bb_engine::mod_corpus`).
//! - `--world-size` makes the campaigns' maps that many tiles across and down instead of
//!   vanilla's 140x170 (vanilla's worldgen needs more than 130x130).
//! - `--shot` saves the frame after that many frames and quits; `--at` runs the Squirrel
//!   `CODE` after that many frames, and `--at-page` the `JS` in the UI page (for scripted
//!   runs; the page's then runs after the frame's Squirrel).
//! - `--settings` reads and keeps the settings in that file instead of the system's
//!   place for them.
//! - The saves (and the mods' storages) are kept in the original game's own savegames
//!   folder, so the player has the same saves in both: on Windows `Battle
//!   Brothers\savegames` in their Documents, on Linux the same in the Proton prefix of
//!   the Steam install played (if the original has run there). Otherwise (macOS, an
//!   install that isn't Steam's on Linux) `battle-brothers-modern/savegames` in the
//!   user's data folder. `--saves DIR`, else `BB_SAVES_DIR`, else `saves.dir` in the
//!   settings names another folder. The log says which and why (see `bb_app::saves`).
//!   Only one bb at a time plays from a folder: a second one started on it says Battle
//!   Brothers is already running, in the message box too, and ends
//!   (`bb_app::saves::lock`).
//! - The game logs to `log.html` in the original's format (and CEF to `cef.log`), in
//!   `battle-brothers-modern/` in the user's data folder, or the folder `--logs` names;
//!   the earlier sessions' logs are kept there in `logs/`, zipped. The terminal gets
//!   the lines too.
//! - The sound plays on the default output device, unless `--mute`.
//! - `--fps` shows the frame rate in the top-left corner (vanilla's own FPS module,
//!   which the original never shows), as `ui.show_fps = true` in the settings does;
//!   Ctrl+Shift+F shows or hides it while playing. It's off by default. It counts the
//!   frames presented, as Steam's overlay does (a still screen, presented once, reads 0
//!   or near it; the display's refresh rate while the mouse moves over it), with the game's steps a second beside them when they differ: `20 (144)`
//!   (see `bb_app::fps`). The scripts' `Time.getFPS` is the steps'.
//! - The game's mouse cursor shows as the system's cursor (the compositor moves it, so
//!   moving the mouse over a still picture draws nothing, though it presents the last
//!   frame again every refresh, for variable-refresh displays: `bb_app::mouse_refresh`);
//!   `ui.software_cursor = true`
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
//! - Ctrl+Shift+F11 (the game doesn't see it), or `SIGUSR1` sent to the game's process on
//!   Linux and macOS, writes a memory snapshot for analysis into `memory-<time>/` in the
//!   log folder, and log.html says where (`bb_app::memory_snapshot`, `docs/memory-snapshots.md`).
//!   `--memory-snapshot-v8` adds V8's heap snapshot of the page (100 MB and more, with the
//!   page's text), with the `ui` feature.
//! - Profiling the frame (see `parity/perf/frame_breakdown.py`): `BB_FRAME_TIMES=PATH` writes
//!   each frame's time by span (the step's parts, the draw's, the page's, the bridge's calls)
//!   to that file; with `BB_GPU_TIMES=1` too, each GPU submission's time (timestamp
//!   queries; the frame then waits for the GPU). `BB_SCRIPT_PROFILE=REPORT.md` profiles the
//!   scripts. Under a preloaded jemalloc with profiling on, `BB_HEAP_PROFILE_DIR=DIR` dumps a
//!   heap profile every `BB_HEAP_PROFILE_SECS` (10) seconds (builds without `ui`).
//! - `--version` prints the version, the target it was built for and whether it has the
//!   UI (`bb 0.2.0 (x86_64-unknown-linux-gnu, ui)`), and exits. It needs no display, but
//!   a UI build has loaded CEF's library by then: the release bundles' smoke check.
//! - Every environment variable, these and the ones for measuring, comparing and the
//!   tests: `docs/environment.md`.
//! - On Windows bb is a GUI program, in every build (debug ones too, so what a developer
//!   runs is what a player double-clicks): no console window opens with it. Run from a
//!   console, it writes to that console, and redirected output stays redirected
//!   (`bb_app::win_process::console`); the processes it starts end with it, and a native
//!   crash is logged and shown (`bb_app::win_process`).

// No console window with the game (ignored elsewhere).
#![windows_subsystem = "windows"]

mod app;
mod window_mode;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bb_app::failure::When;
use bb_app::game::Start;
use bb_app::steam::SteamOptions;
use bb_app::install::Asked;
use bb_app::settings::Settings;
use bb_engine::install::NotFound;
use winit::event_loop::EventLoop;

/// The map seed of a campaign started at once, unless `--seed` gives one.
const DEFAULT_SEED: &str = "rust-engine-test";

/// What the command line names for the lookup (`bb_app::install`).
#[derive(Default)]
struct DataArguments {
	/// `--data`'s folder.
	data: Option<PathBuf>,
	/// `--chosen-data`'s: the folder the player chose in the not-found window, when it
	/// couldn't be kept in the settings (bb passes it to itself as it starts again). It
	/// stands for the folder chosen before, so it isn't the command line's.
	chosen: Option<PathBuf>,
}

/// Why a run ended in error.
enum Failure {
	/// The command line's (see the module's notes): on stderr only.
	Arguments(String),
	/// Anything else, as bb started or once the game ran: in the message box too.
	Game(String, When),
	/// A start bb refused, in these words, in the message box too: another bb plays
	/// from the same saves.
	Refused(String),
}

impl From<String> for Failure {
	fn from(error: String) -> Failure {
		Failure::Game(error, When::Starting)
	}
}

/// The options, Steam's, the log folder (`None`: the system has no data folder), and
/// the folders for the lookup. The options' data folder is left for the lookup.
fn options() -> Result<(app::Options, SteamOptions, Option<PathBuf>, DataArguments), String> {
	let mut steam = SteamOptions::from_env();
	let mut data = DataArguments::default();
	let mut options = app::Options {
		data_dir: PathBuf::new(),
		// Without the page there's no menu to click.
		start: if cfg!(feature = "ui") { Start::Menu } else { Start::NewCampaign(DEFAULT_SEED.to_string()) },
		mods: std::env::var_os("BB_NO_MODS").is_none_or(|v| v == "0"),
		mods_dir: None,
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
		logs_dir: None,
		memory_snapshot_v8: false,
	};
	let mut logs = squirrel_bb::platform::logs_dir();
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().ok_or(format!("{arg} needs a value"));
		match arg.as_str() {
			"--data" => data.data = Some(PathBuf::from(value()?)),
			"--chosen-data" => data.chosen = Some(PathBuf::from(value()?)),
			"--seed" => options.start = Start::NewCampaign(value()?),
			"--new-campaign" => {
				if let Start::Menu = options.start {
					options.start = Start::NewCampaign(DEFAULT_SEED.to_string());
				}
			}
			"--mods" => options.mods = true,
			"--no-mods" => options.mods = false,
			"--mods-dir" => {
				options.mods_dir = Some(PathBuf::from(value()?));
				options.mods = true;
			}
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
			"--memory-snapshot-v8" if cfg!(feature = "ui") => options.memory_snapshot_v8 = true,
			"--memory-snapshot-v8" => return Err("--memory-snapshot-v8 needs bb built with --features ui".to_string()),
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
	// Every process of bb's, the browser's too, before anything else (see
	// `bb_app::memory_snapshot::trigger`): the game's listens once it runs.
	bb_app::memory_snapshot::trigger::ignore_signal();
	bb_app::platform::allow_tracing_if_asked();
	// The browser host re-runs this binary for its child processes.
	#[cfg(feature = "ui")]
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// A GUI program on Windows (no console opens when it's double-clicked): its output
	// goes to the console it was run from, if any (`bb_app::win_process::console`).
	#[cfg(windows)]
	bb_app::win_process::console::attach();
	if std::env::args().nth(1).as_deref() == Some("--version") {
		println!("{}", bb_app::version());
		return ExitCode::SUCCESS;
	}
	bb_app::platform::tune_allocator();
	squirrel_bb::startup::mark("main");
	let mut logs = None;
	// The game's data folder, once found (the failure message names it).
	let mut data_dir = None;
	let mut run = || -> Result<Ended, Failure> {
		let (mut options, steam, log_dir, data) = options().map_err(Failure::Arguments)?;
		if let Some(dir) = log_dir {
			match bb_app::logs::Logs::start(&dir) {
				Ok(started) => {
					options.cef_log = Some(started.cef_log());
					options.logs_dir = started.dir().map(Path::to_path_buf);
					logs = Some(started);
				}
				Err(e) => eprintln!("no log file: {e}"),
			}
		}
		bb_app::failure::show_panics(logs.as_ref().and_then(bb_app::logs::Logs::dir).map(Path::to_path_buf));
		#[cfg(windows)]
		{
			bb_app::win_process::crash::report_crashes(logs.as_ref().and_then(bb_app::logs::Logs::dir).map(Path::to_path_buf));
			// Before any process of bb's starts (CEF's, a relaunch).
			if let Err(e) = bb_app::win_process::job::end_children_with_bb() {
				squirrel_bb::html_log::engine(squirrel_bb::html_log::Class::Warning, "Core", &format!("bb's processes may outlive it: {e}"));
			}
		}
		let mut settings = load_settings(&options);
		let chosen = data.chosen.or_else(|| settings.chosen_data_dir());
		options.data_dir = match bb_app::install::find(data.data, chosen) {
			Ok(install) => install.data_dir,
			Err(NotFound::Nowhere(looked)) => {
				let picker = picker_options(&options);
				return match bb_app::install::ask(&looked, &mut settings, picker)? {
					Asked::Closed => Ok(Ended::Closed),
					asked => Ok(Ended::Relaunch(bb_app::install::relaunch_arguments(&asked))),
				};
			}
			// `--data` or `BB_DATA_DIR`: neither is a double-clicking player's.
			Err(bad @ NotFound::BadOverride(_)) => return Err(Failure::Arguments(bad.to_string())),
		};
		data_dir = Some(options.data_dir.clone());
		options.saves = bb_app::saves::choose_for(options.saves.take(), &settings, &options.data_dir).map(|c| c.dir);
		// One bb at a time on a saves folder, held until bb ends, taken before CEF starts.
		// A relaunch (the not-found window's) ended before here, so its new bb isn't
		// refused by the one that started it.
		let _saves_lock = match &options.saves {
			Some(dir) => match bb_app::saves::lock::SavesLock::take(dir) {
				Ok(Some(lock)) => Some(lock),
				Ok(None) => return Err(Failure::Refused(already_running(dir))),
				Err(e) => {
					let line = format!("saves: no lock on {}, so another bb could use it too: {e}", dir.display());
					squirrel_bb::html_log::engine(squirrel_bb::html_log::Class::Warning, "Saves", &line);
					None
				}
			},
			None => None,
		};
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
		app.error.take().map_or(Ok(Ended::Played), |(e, when)| Err(Failure::Game(e, when)))
	};
	let result = run();
	let log_dir = logs.as_ref().and_then(bb_app::logs::Logs::dir).map(Path::to_path_buf);
	// Every failure a player can meet is told to them in words (a machine that can't run
	// the game with the requirements), not only in the log and on stderr.
	let message = match &result {
		Err(Failure::Game(e, when)) => Some(bb_app::failure::message(e, *when, data_dir.as_deref(), log_dir.as_deref())),
		Err(Failure::Arguments(e) | Failure::Refused(e)) => Some(e.clone()),
		Ok(_) => None,
	};
	if let Some(logs) = logs {
		logs.close(message.as_deref());
	}
	let fail = |title: &str, text: &str| {
		eprintln!("\n{text}");
		bb_app::requirements::dialog::show(title, text);
		ExitCode::FAILURE
	};
	match result {
		Ok(Ended::Played | Ended::Closed) => ExitCode::SUCCESS,
		Ok(Ended::Relaunch(extra)) => match bb_app::install::relaunch::relaunch(&extra) {
			Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
			Err(e) => fail(
				bb_app::failure::title(&e, When::Starting),
				&bb_app::failure::message(&e, When::Starting, data_dir.as_deref(), log_dir.as_deref()),
			),
		},
		Err(Failure::Game(e, when)) => {
			eprintln!("error: {e}");
			fail(bb_app::failure::title(&e, when), message.as_deref().unwrap_or(&e))
		}
		Err(Failure::Arguments(e)) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		}
		Err(Failure::Refused(e)) => fail(bb_app::failure::title(&e, When::Starting), &e),
	}
}

/// The player's message when another bb plays from the saves in `dir`.
fn already_running(dir: &Path) -> String {
	format!(
		"Battle Brothers is already running, with its saves in {}.\n\nOnly one game at a time can use \
		 those saves: switch to the one that's running, or close it and start the game again.",
		dir.display()
	)
}
