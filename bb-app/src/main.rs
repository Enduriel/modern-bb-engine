//! Battle Brothers on the modern engine, in a window.
//!
//! ```text
//! cargo run --release -p bb-app --features ui -- [--mods] [--new-campaign] [--seed SEED]
//!     [--data DIR] [--world-size COLSxROWS] [--shot FRAMES OUT.png] [--mute]
//!     [--devtools PORT] [--settings PATH] [--saves DIR] [--at FRAMES CODE]...
//!     [--no-steam] [--steam-api PATH]
//! ```
//!
//! Boots the install's game files (the Steam install by default) and plays them: the
//! main menu, the world map or the battle, the mouse and keys going to the game.
//!
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
//!   `CODE` after that many frames (for scripted runs).
//! - `--settings` reads and keeps the settings in that file instead of the system's
//!   place for them, and `--saves` keeps the saves in that folder instead of the
//!   system's place for them (`battle-brothers-modern/savegames` in the user's data
//!   folder; never the original's).
//! - The sound plays on the default output device, unless `--mute`.
//! - On a Steam install (see `bb_steam::install` for the rule), achievements and stats
//!   go to the player's Steam account through Steam's library beside `bb`
//!   (`libsteam_api.so`, `steam_api64.dll` or `libsteam_api.dylib`), or the one
//!   `--steam-api` (or `BB_STEAM_API`) names. `--no-steam` (or `BB_NO_STEAM=1`) never
//!   loads it. Any other install never does either, and without Steam the game runs on
//!   with achievements off (said once in the log).
//! - With `BB_STATS=1` the game's total CPU and memory, its CEF processes included, is
//!   logged every 5 seconds.

mod app;
mod window_mode;

use std::path::PathBuf;
use std::process::ExitCode;

use bb_app::game::Start;
use bb_app::steam::SteamOptions;
use bb_engine::harness::STEAM_DATA_DIR;
use winit::event_loop::EventLoop;

/// The map seed of a campaign started at once, unless `--seed` gives one.
const DEFAULT_SEED: &str = "rust-engine-test";

fn options() -> Result<(app::Options, SteamOptions), String> {
	let mut steam = SteamOptions::from_env();
	let mut options = app::Options {
		data_dir: PathBuf::from(STEAM_DATA_DIR),
		// Without the page there's no menu to click.
		start: if cfg!(feature = "ui") { Start::Menu } else { Start::NewCampaign(DEFAULT_SEED.to_string()) },
		mods: false,
		shot: None,
		mute: false,
		world_size: None,
		devtools: None,
		settings: None,
		at: Vec::new(),
		saves: None,
		steam_build: false,
		steam: None,
	};
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().ok_or(format!("{arg} needs a value"));
		match arg.as_str() {
			"--data" => options.data_dir = PathBuf::from(value()?),
			"--seed" => options.start = Start::NewCampaign(value()?),
			"--new-campaign" => {
				if let Start::Menu = options.start {
					options.start = Start::NewCampaign(DEFAULT_SEED.to_string());
				}
			}
			"--mods" => options.mods = true,
			"--mute" => options.mute = true,
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
			"--at" => {
				let frames = value()?.parse().map_err(|_| "--at FRAMES CODE")?;
				options.at.push((frames, value()?));
			}
			"--shot" => {
				let frames = value()?.parse().map_err(|_| "--shot FRAMES OUT.png")?;
				options.shot = Some((frames, PathBuf::from(value()?)));
			}
			_ => return Err(format!("unknown argument {arg}")),
		}
	}
	Ok((options, steam))
}

fn main() -> ExitCode {
	// The browser host re-runs this binary for its child processes.
	#[cfg(feature = "ui")]
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	bb_app::platform::tune_allocator();
	let run = || -> Result<(), String> {
		let (mut options, steam) = options()?;
		options.steam_build = bb_app::steam::is_steam_install(&options.data_dir);
		// SAFETY: no other thread runs yet (Steam's start sets the app id in the
		// environment): the stats logger, the renderer's and the page's start after it.
		options.steam = unsafe { bb_app::steam::start(&options.data_dir, &steam) };
		// The game's total CPU and memory, with its CEF processes, every 5 s.
		if std::env::var_os("BB_STATS").is_some_and(|v| v != "0") {
			bb_app::stats::spawn_logger(std::time::Duration::from_secs(5));
		}
		let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
		let display = Box::new(event_loop.owned_display_handle());
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(display));
		let mut app = app::App::new(options, instance);
		event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
		app.error.take().map_or(Ok(()), Err)
	};
	match run() {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		}
	}
}
