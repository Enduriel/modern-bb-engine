//! Battle Brothers on the modern engine, in a window.
//!
//! ```text
//! cargo run --release -p bb-app -- [--data DIR] [--seed SEED] [--shot FRAMES OUT.png]
//! ```
//!
//! Boots the install's game files (the Steam install by default) into a new campaign
//! and plays it: the world map or the battle, the mouse and keys going to the game.
//! `--shot` saves the frame after that many frames and quits. Built with `--features
//! ui`, the game's real UI is drawn over it and takes the input it's under.

mod app;

use std::path::PathBuf;
use std::process::ExitCode;

use bb_engine::harness::STEAM_DATA_DIR;
use winit::event_loop::EventLoop;

fn options() -> Result<app::Options, String> {
	let mut options = app::Options {
		data_dir: PathBuf::from(STEAM_DATA_DIR),
		seed: "rust-engine-test".to_string(),
		shot: None,
	};
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().ok_or(format!("{arg} needs a value"));
		match arg.as_str() {
			"--data" => options.data_dir = PathBuf::from(value()?),
			"--seed" => options.seed = value()?,
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
	let run = || -> Result<(), String> {
		let options = options()?;
		let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
		let display = Box::new(event_loop.owned_display_handle());
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(display));
		let mut app = app::App::new(options, instance);
		event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
		app.error.map_or(Ok(()), Err)
	};
	match run() {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		}
	}
}
