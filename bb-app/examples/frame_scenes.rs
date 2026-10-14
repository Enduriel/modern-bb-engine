//! Frame parity, our side: the scenes of `parity/frames/scenes.{nut,js}`, which the
//! original screenshots in a probe, drawn offscreen as `bb` draws them, into PNGs to
//! diff with the original's (`bb-render`'s `frame_diff`).
//!
//! ```text
//! cargo run --release -p bb-app --features ui --example frame_scenes -- OUT_DIR [--screen WxH] [SCENE...]
//! ```
//!
//! The campaign is the probe's (seed `bbuitest`), its scene script runs where the probe
//! runs it (right after `world_state.init()`), the intro event is closed with "Huzzah!",
//! and each scene is set up by the script's `probeScene` and given 3 s, as the probe's
//! page does. The screen is the original's video mode, which a probe can't change
//! (default 2560x1440, at UI scale 1). The drawn cursor is left out: the original's
//! screenshots have none.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_app::game::Game;
use bb_engine::harness::STEAM_DATA_DIR;
use bb_render::{Gpu, RenderTarget, save_png};
use squirrel_bb::VideoMode;

const SEED: &str = "bbuitest";
const STEP: Duration = Duration::from_secs(180);

/// The scenes' script, the probe's own.
fn scenes_script() -> Result<String, String> {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/frames/scenes.nut");
	std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Run `seconds` of frames at 60 a second, as the game loop does, drawing each.
fn play(game: &mut Game, gpu: &Gpu, target: &RenderTarget, seconds: f64) -> Result<(), String> {
	let frame = Duration::from_micros(16_667);
	for _ in 0..(seconds * 60.0) as u32 {
		let start = Instant::now();
		game.step(1000.0 / 60.0)?;
		game.draw(gpu, target);
		if let Some(rest) = frame.checked_sub(start.elapsed()) {
			std::thread::sleep(rest);
		}
	}
	Ok(())
}

fn run() -> Result<(), String> {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let mut out = None;
	let mut screen = (2560i64, 1440i64);
	let mut scenes = Vec::new();
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
			a => scenes.push(a.to_string()),
		}
		i += 1;
	}
	let out = out.ok_or("usage: frame_scenes OUT_DIR [--screen WxH] [SCENE...]")?;
	if scenes.is_empty() {
		scenes.push("world".to_string());
	}
	std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;

	let gpu = Gpu::headless()?;
	let mode = VideoMode {
		width: screen.0,
		height: screen.1,
		fullscreen: false,
		borderless: true,
		ui_scale: 1.0,
		scene_scale: 1.0,
		depth_of_field: false,
		..VideoMode::DEFAULT
	};
	let mut game = Game::boot(
		&gpu,
		Path::new(STEAM_DATA_DIR),
		SEED,
		false,
		None,
		mode,
		None,
	)?;
	{
		let session = game.front().session();
		let engine = session.engine();
		// Where the probe harness runs it: right after world_state.init(), before the
		// world's first update (see bb-ui's goldens::capture_world).
		let deadline = Instant::now() + STEP;
		loop {
			engine.init_pending_states()?;
			if engine.root_state().state("WorldState").is_some() {
				engine.run_script(&scenes_script()?, "scenes.nut")?;
				break;
			}
			if Instant::now() >= deadline {
				return Err("the campaign's world never started".into());
			}
			session.run_frames(0.016)?;
		}
		session.wait_for_text("Huzzah!", STEP)?;
		session.run_frames(3.0)?;
		session.click_when_ready("Huzzah!", STEP)?;
		// The probe clicks with jQuery: no mouse is over its page.
		session.move_away()?;
		engine.run_script("::UI.setCursorHardware(true);", "no drawn cursor")?;
	}

	let (width, height) = game.screen();
	let target = RenderTarget::new(&gpu, width, height);
	play(&mut game, &gpu, &target, 3.0)?;
	for scene in &scenes {
		game.front().engine().run_script(
			&format!("::World.State.m.WorldScreen.probeScene(\"{scene}\");"),
			"scene",
		)?;
		play(&mut game, &gpu, &target, 3.0)?;
		let frame = target.read(&gpu)?;
		let path = out.join(format!("{scene}.png"));
		save_png(&path, &frame)?;
		println!("{scene}: {}", path.display());
	}
	Ok(())
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
