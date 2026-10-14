//! Battle Brothers on the modern engine, in a window.
//!
//! ```text
//! cargo run --release -p bb-app -- [--data DIR] [--seed SEED] [--world-size COLSxROWS]
//!                                    [--shot FRAMES OUT.png] [--mute] [--devtools PORT]
//!                                    [--settings PATH] [--at FRAMES CODE]...
//! ```
//!
//! Boots the install's game files (the Steam install by default) into a new campaign
//! and plays it: the world map or the battle, the mouse and keys going to the game.
//! `--world-size` makes the map that many tiles across and down instead of vanilla's
//! 140x170 (vanilla's worldgen needs more than 130x130). `--shot` saves the frame after
//! that many frames and quits; `--at` runs the Squirrel `CODE` after that many frames
//! (for scripted runs). `--settings` reads and keeps the settings in that file instead
//! of the system's place for them. Built with `--features
//! ui`, the game's real UI is drawn over it and takes the input it's under, and
//! `--devtools` serves Chrome DevTools for its page on localhost at that port. The sound
//! plays on the default output device, unless `--mute`. With `BB_STATS=1` the game's
//! total CPU and memory, its CEF processes included, is logged every 5 seconds.

mod app;
mod window_mode;

use std::path::PathBuf;
use std::process::ExitCode;

use bb_engine::harness::STEAM_DATA_DIR;
use winit::event_loop::EventLoop;

fn options() -> Result<app::Options, String> {
	let mut options = app::Options {
		data_dir: PathBuf::from(STEAM_DATA_DIR),
		seed: "rust-engine-test".to_string(),
		shot: None,
		mute: false,
		world_size: None,
		devtools: None,
		settings: None,
		at: Vec::new(),
	};
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().ok_or(format!("{arg} needs a value"));
		match arg.as_str() {
			"--data" => options.data_dir = PathBuf::from(value()?),
			"--seed" => options.seed = value()?,
			"--mute" => options.mute = true,
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
	Ok(options)
}

fn main() -> ExitCode {
	// The browser host re-runs this binary for its child processes.
	#[cfg(feature = "ui")]
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	bb_app::platform::tune_allocator();
	// The game's total CPU and memory, with its CEF processes, every 5 s.
	if std::env::var_os("BB_STATS").is_some_and(|v| v != "0") {
		bb_app::stats::spawn_logger(std::time::Duration::from_secs(5));
	}
	let run = || -> Result<(), String> {
		let options = options()?;
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
