//! Frame parity, our side: the scenes of `parity/frames/scenes.{nut,js}`, which the
//! original screenshots in a probe, drawn offscreen as `bb` draws them
//! ([`bb_app::frame_scenes`]), into PNGs to diff with the original's (`bb-render`'s
//! `frame_diff`).
//!
//! ```text
//! cargo run --release -p bb-app --features ui --example frame_scenes -- OUT_DIR [--screen WxH] [--scenes FILE]
//! ```
//!
//! `--scenes` walks another scene script than `parity/frames/scenes.nut` (a probe's
//! own, written the same way).
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
	let mut scenes = bb_app::frame_scenes::default_scenes();
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
			"--scenes" => {
				i += 1;
				scenes = PathBuf::from(args.get(i).ok_or("--scenes FILE")?);
			}
			a if out.is_none() => out = Some(PathBuf::from(a)),
			a => return Err(format!("unknown argument {a}")),
		}
		i += 1;
	}
	let out = out.ok_or("usage: frame_scenes OUT_DIR [--screen WxH] [--scenes FILE]")?;
	std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
	let gpu = Gpu::headless_with(bb_app::front::GPU_FEATURES)?;
	bb_app::frame_scenes::shoot_scenes(&gpu, screen, &scenes, |name, image| {
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
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	match run() {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		}
	}
}
