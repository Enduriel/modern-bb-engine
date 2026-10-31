//! What a held key costs the game's frames: the game as `bb` runs it, offscreen, on a
//! new campaign's world map (running), paced at a display's refresh. Each scene runs
//! `SECONDS`: no key, then `W` held (the map scrolls; the page gets its keydown every
//! frame, as the original's), then `W` released, then `Alt` held alone.
//!
//! ```text
//! cargo run --release -p bb-app --features ui --example held_key -- [HZ [SECONDS]]
//! ```
//!
//! Per scene: the frames' step time (scripts, the page's hand-off and turn) and draw
//! time (mean, median, 95th percentile), and the frames the page rested through.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_app::front::Key;
use bb_app::game::{Boot, Game, Start, Store};
use bb_engine::harness::STEAM_DATA_DIR;
use bb_engine::tactical_player as player;
use bb_render::{Gpu, RenderTarget};
use squirrel_bb::VideoMode;

const IN_WORLD: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";

/// The engine's codes: `W`, Alt.
const W: i64 = 33;
const ALT: i64 = 97;

struct Run<'a> {
	game: Game,
	gpu: &'a Gpu,
	target: RenderTarget,
	interval: Duration,
	next: Instant,
}

impl Run<'_> {
	fn holds(&self, cond: &str) -> bool {
		player::eval_i64(self.game.front().engine(), &format!("({cond}) ? 1 : 0")).ok().flatten() == Some(1)
	}

	/// One frame, paced: its step and draw times.
	fn frame(&mut self) -> Result<(Duration, Duration), String> {
		let ms = self.interval.as_secs_f64() * 1000.0;
		let start = Instant::now();
		self.game.step(ms)?;
		let stepped = Instant::now();
		self.game.draw_if_changed(self.gpu, &self.target);
		self.gpu.device.poll(wgpu::PollType::Poll).map_err(|e| e.to_string())?;
		let drawn = Instant::now();
		self.next += self.interval;
		match self.next.checked_duration_since(drawn) {
			Some(wait) => std::thread::sleep(wait),
			None => self.next = drawn,
		}
		Ok((stepped - start, drawn - stepped))
	}

	fn scene(&mut self, name: &str, seconds: f64) -> Result<(), String> {
		let frames = (seconds / self.interval.as_secs_f64()) as usize;
		let rested = self.game.front().session().host().rested_frames();
		let (mut steps, mut draws) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
		for _ in 0..frames {
			let (step, draw) = self.frame()?;
			steps.push(step);
			draws.push(draw);
		}
		let rested = self.game.front().session().host().rested_frames() - rested;
		println!("== {name}: {frames} frames, the page rested through {rested}");
		println!("   step {}", stats(&mut steps));
		println!("   draw {}", stats(&mut draws));
		Ok(())
	}

	fn key(&mut self, engine: i64, text: Option<&str>, down: bool) -> Result<(), String> {
		self.game.key(&Key { engine: Some(engine), text: text.map(str::to_string), down, repeat: false })
	}
}

/// Mean, median and 95th percentile, in ms.
fn stats(times: &mut [Duration]) -> String {
	times.sort_unstable();
	let ms = |d: Duration| d.as_secs_f64() * 1000.0;
	let mean = times.iter().map(|&d| ms(d)).sum::<f64>() / times.len().max(1) as f64;
	let at = |q: f64| times.get(((times.len() as f64 * q) as usize).min(times.len().saturating_sub(1))).map_or(0.0, |&d| ms(d));
	format!("mean {mean:.3} ms, median {:.3}, p95 {:.3}, max {:.3}", at(0.5), at(0.95), at(1.0))
}

fn run(hz: f64, seconds: f64) -> Result<(), String> {
	let gpu = Gpu::headless_with(bb_app::front::GPU_FEATURES)?;
	let boot = Boot {
		data_dir: Path::new(STEAM_DATA_DIR),
		start: Start::NewCampaign("heldkey".into()),
		mods: None,
		sound: false,
		world_size: None,
		mode: VideoMode { width: 2560, height: 1358, refresh_rate: hz.round() as i64, ..VideoMode::DEFAULT },
		settings: None,
		devtools: None,
		saves: None,
		cef_log: None,
		store: Store::default(),
		update_check: None,
		display: None,
	};
	let game = Game::start(&gpu, &boot)?;
	let (w, h) = game.screen();
	let target = RenderTarget::new(&gpu, w, h);
	let interval = Duration::from_secs_f64(1.0 / hz);
	let mut run = Run { game, gpu: &gpu, target, interval, next: Instant::now() };
	let since = Instant::now();
	while !run.holds(IN_WORLD) {
		run.frame()?;
		if since.elapsed() > Duration::from_secs(120) {
			return Err("the campaign's world never showed".into());
		}
	}
	// The origin's intro event answered (as its first button), the world running.
	run.game.run_script("::World.State.m.IsAutosaving = false;")?;
	const ANSWER: &str = "local s = ::World.State.getEventScreen(); if (s.isVisible() && !s.isAnimating()) s.onButtonPressed(0);";
	let since = Instant::now();
	let mut closed = 0;
	// The event comes a little after the world shows: answered once it has been gone a
	// second.
	while closed < 180 {
		if run.holds("::World.State.getEventScreen().isVisible() || ::World.Events.hasActiveEvent()") {
			closed = 0;
			run.game.run_script(ANSWER)?;
		} else {
			closed += 1;
		}
		run.frame()?;
		if since.elapsed() > Duration::from_secs(60) {
			return Err("the origin's intro event never closed".into());
		}
	}
	run.game.run_script("::World.State.setAutoPause(false); ::World.State.setPause(false);")?;
	run.scene("settling", 3.0)?;
	run.scene("no key", seconds)?;
	let camera = |run: &Run| player::eval_i64(run.game.front().engine(), "::World.getCamera().getPos().Y.tointeger()").ok().flatten();
	let before = camera(&run);
	run.key(W, Some("w"), true)?;
	run.scene("W held", seconds)?;
	println!("   the camera's Y went from {before:?} to {:?}", camera(&run));
	run.key(W, None, false)?;
	run.scene("W released", seconds)?;
	run.key(ALT, None, true)?;
	run.scene("Alt held", seconds)?;
	run.key(ALT, None, false)?;
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let args: Vec<f64> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
	let hz = args.first().copied().unwrap_or(180.0);
	let seconds = args.get(1).copied().unwrap_or(5.0);
	match run(hz, seconds) {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("held_key: {e}");
			ExitCode::FAILURE
		}
	}
}
