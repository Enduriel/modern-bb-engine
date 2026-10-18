//! Frame parity, our side: the scenes of `parity/frames/scenes.{nut,js}`, which the
//! original screenshots in a probe, drawn offscreen as `bb` draws them
//! ([`bb_app::frame_scenes`]), into PNGs to diff with the original's (`bb-render`'s
//! `frame_diff`).
//!
//! ```text
//! cargo run --release -p bb-app --features ui --example frame_scenes -- OUT_DIR [--screen WxH]
//! ```
//!
//! The screen is the original's video mode, which a probe can't change (default
//! 2560x1440, at UI scale 1).

use std::path::PathBuf;
use std::process::ExitCode;

use bb_render::{Gpu, save_png};

fn run() -> Result<(), String> {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let mut out = None;
	let mut screen = (2560i64, 1440i64);
	let mut i = 0;
	while i < args.len() {
		match args[i].as_str() {
			"--screen" => {
				i += 1;
				let v = args.get(i).ok_or("--screen WxH")?;
				screen = v
					.split_once('x')
					.and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
					.ok_or(format!("--screen WxH, not {v}"))?;
			}
			a if out.is_none() => out = Some(PathBuf::from(a)),
			a => return Err(format!("unknown argument {a}")),
		}
		i += 1;
	}
	let out = out.ok_or("usage: frame_scenes OUT_DIR [--screen WxH]")?;
	std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
	let gpu = Gpu::headless()?;
	bb_app::frame_scenes::shoot(&gpu, screen, |name, image| {
		let path = out.join(format!("{name}.png"));
		save_png(&path, &image)?;
		println!("{name}: {}", path.display());
		Ok(())
	})
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	match run() {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		}
	}
}
