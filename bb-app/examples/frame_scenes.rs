//! Frame parity, our side: the scenes of `parity/frames/scenes.{nut,js}`, which the
//! original screenshots in a probe, drawn offscreen as `bb` draws them, into PNGs to
//! diff with the original's (`bb-render`'s `frame_diff`).
//!
//! ```text
//! cargo run --release -p bb-app --features ui --example frame_scenes -- OUT_DIR [--screen WxH]
//! ```
//!
//! The campaign is the probe's (seed `bbuitest`), and its scene script runs where the
//! probe runs it (right after `world_state.init()`). Once the intro event shows, the
//! script's `::probeSteps` are walked as the probe's page walks them: each step's scene
//! set up, its `until` polled, 3 s to settle, its shot saved, its button clicked. The
//! screen is the original's video mode, which a probe can't change (default 2560x1440,
//! at UI scale 1). The drawn cursor is left out: the original's screenshots have none.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_app::game::Game;
use bb_engine::harness::STEAM_DATA_DIR;
use bb_render::{Gpu, RenderTarget, save_png};
use squirrel::SqTable;
use squirrel_bb::VideoMode;

const SEED: &str = "bbuitest";
const STEP: Duration = Duration::from_secs(180);

/// The scenes' script, the probe's own.
fn scenes_script() -> Result<String, String> {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/frames/scenes.nut");
	std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// One of the script's `::probeSteps`.
struct Step {
	scene: Option<String>,
	shot: Option<String>,
	until: Option<String>,
	click: Option<String>,
}

/// The script's `::probeSteps`.
fn steps(game: &Game) -> Result<Vec<Step>, String> {
	let engine = game.front().engine();
	engine.run_script(
		r#"::__steps <- "";
		foreach (s in ::probeSteps)
			foreach (k in ["scene", "shot", "until", "click"])
				::__steps += (s[k] == null ? "" : s[k]) + (k == "click" ? "\n" : "\t");"#,
		"steps",
	)?;
	let text = SqTable::root(engine.vm()).get::<String>("__steps").ok_or("no steps")?;
	let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
	Ok(text
		.lines()
		.map(|line| {
			let f: Vec<&str> = line.split('\t').collect();
			Step {
				scene: some(f[0]),
				shot: some(f[1]),
				until: some(f[2]),
				click: some(f[3]),
			}
		})
		.collect())
}

/// Call the script's `probeScene(name)`; whether it returned true.
fn scene(game: &Game, name: &str) -> Result<bool, String> {
	let engine = game.front().engine();
	engine.run_script(
		&format!("::__scene <- ::World.State.m.WorldScreen.probeScene(\"{name}\") == true;"),
		"scene",
	)?;
	Ok(SqTable::root(engine.vm()).get::<bool>("__scene") == Some(true))
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
		// No mouse is over the probe's page: it clicks with jQuery.
		session.move_away()?;
		engine.run_script("::UI.setCursorHardware(true);", "no drawn cursor")?;
	}

	let (width, height) = game.screen();
	let target = RenderTarget::new(&gpu, width, height);
	for step in steps(&game)? {
		if let Some(name) = &step.scene {
			scene(&game, name)?;
		}
		if let Some(until) = &step.until {
			let deadline = Instant::now() + STEP;
			while !scene(&game, until)? {
				if Instant::now() >= deadline {
					return Err(format!("{until} never came true"));
				}
				play(&mut game, &gpu, &target, 0.5)?;
			}
		}
		play(&mut game, &gpu, &target, 3.0)?;
		if let Some(shot) = &step.shot {
			let path = out.join(format!("{shot}.png"));
			save_png(&path, &target.read(&gpu)?)?;
			println!("{shot}: {}", path.display());
		}
		if let Some(label) = &step.click {
			let session = game.front().session();
			session.click_when_ready(label, STEP)?;
			session.move_away()?;
			play(&mut game, &gpu, &target, 3.0)?;
		}
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
