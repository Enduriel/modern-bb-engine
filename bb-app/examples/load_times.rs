//! How long the game takes to show each scene, and whether anything pops in after it
//! shows: the game as `bb` plays it, offscreen, with its real page.
//!
//! ```text
//! cargo run --release -p bb-app --features ui --example load_times -- menu
//! cargo run --release -p bb-app --features ui --example load_times -- game
//! ```
//!
//! - `menu`: the page and the scripts started, to the main menu shown and settled.
//! - `game`: from the start, the first campaign's world drawn whole; then a new campaign
//!   from the main menu, a load (a save made of that world, as the in-game menu loads),
//!   and a battle's start.
//!
//! A scene counts as shown at the first frame, once its loading screen is gone, that
//! draws every quad: none skipped because its atlas is still loading. Each frame drawn
//! after the loading screen with quads still waiting is pop-in, and is counted. Times
//! are wall-clock seconds; the frames run as fast as they draw, at a 60 Hz game step.
//! The engine reads no preload lists (`bb-assets` loader): atlases load when a sprite
//! names them or a draw needs them.

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use bb_app::game::Game;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_engine::tactical_player as player;
use bb_render::{Gpu, RenderTarget};
use squirrel_bb::VideoMode;

const STEP_MS: f64 = 1000.0 / 60.0;
/// Frames a scene may take to show.
const MAX_FRAMES: u32 = 20_000;

const AT_MENU: &str = "::RootState.get(\"WorldState\") == null && ::RootState.get(\"MainMenuState\") != null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible() && !::RootState.get(\"MainMenuState\").m.MainMenuScreen.isAnimating()";
const IN_WORLD: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible() && ::World.isVisible()";
const IN_BATTLE: &str = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible()";

/// A battle next to the company: brigands made by vanilla's spawn list, engaged as the
/// combat dialog offers it (the scripted player clicks Engage).
const ENGAGE: &str = r#"
local player = ::World.State.getPlayer();
local bandits = ::World.FactionManager.getFactionOfType(::Const.FactionType.Bandits);
bandits.spawnEntity(player.getTile(), "Brigand Raiders", false, ::Const.World.Spawn.BanditRaiders, 100);
::__autoplayEngage = true;
::World.State.showCombatDialog(true, true, true, null, player.getPos());
"#;

/// When a scene showed, and the pop-in after its loading screen.
struct Shown {
	seconds: f64,
	/// When the loading screen was first fully shown, and when it was gone again.
	screen_up: Option<f64>,
	screen_gone: Option<f64>,
	frames: u32,
	pop_in_frames: u32,
	most_waiting: usize,
}

struct Run<'a> {
	game: Game,
	gpu: &'a Gpu,
	target: RenderTarget,
	/// Seconds spent in the game's steps, its draws, and this tool's own checks.
	spent: std::cell::Cell<(f64, f64, f64)>,
}

impl Run<'_> {
	fn holds(&self, cond: &str) -> bool {
		let t = Instant::now();
		let holds = player::eval_i64(self.game.front().engine(), &format!("({cond}) ? 1 : 0")).ok().flatten() == Some(1);
		let (a, b, c) = self.spent.get();
		self.spent.set((a, b, c + t.elapsed().as_secs_f64()));
		holds
	}

	/// One frame: the game's step, the scripted player, a drawn frame; its stats.
	fn frame(&mut self) -> Result<bb_render::FrameStats, String> {
		let t = Instant::now();
		self.game.step(STEP_MS)?;
		let stepped = t.elapsed().as_secs_f64();
		let engine = self.game.front().engine();
		if let Some(f) = squirrel::SqTable::root(engine.vm()).get::<squirrel::SqStrongRef>("__autoplay") {
			let _ = f.call(engine.vm(), ());
		}
		let t = Instant::now();
		let stats = self.game.draw(self.gpu, &self.target);
		self.gpu.device.poll(wgpu::PollType::Poll).map_err(|e| e.to_string())?;
		let (a, b, c) = self.spent.get();
		self.spent.set((a + stepped, b + t.elapsed().as_secs_f64(), c));
		Ok(stats)
	}

	/// Frames until `cond` holds and a frame draws every quad, timed from `since`.
	fn until_shown(&mut self, since: Instant, what: &str, cond: &str) -> Result<Shown, String> {
		let (mut frames, mut pop_in_frames, mut most_waiting) = (0, 0, 0);
		let (mut screen_up, mut screen_gone) = (None, None);
		loop {
			let stats = self.frame()?;
			frames += 1;
			let up = self.holds("::LoadingScreen.isVisible() && !::LoadingScreen.isAnimating()");
			if up && screen_up.is_none() {
				screen_up = Some(since.elapsed().as_secs_f64());
			}
			if screen_up.is_some() && screen_gone.is_none() && !self.holds("::LoadingScreen.isVisible()") {
				screen_gone = Some(since.elapsed().as_secs_f64());
			}
			if self.holds(cond) {
				if stats.quads > 0 && stats.waiting == 0 {
					return Ok(Shown {
						seconds: since.elapsed().as_secs_f64(),
						screen_up,
						screen_gone,
						frames,
						pop_in_frames,
						most_waiting,
					});
				}
				if stats.waiting > 0 {
					pop_in_frames += 1;
					most_waiting = most_waiting.max(stats.waiting);
				}
			}
			if frames > MAX_FRAMES {
				let state = player::eval_string(
					self.game.front().engine(),
					"(function() { return \"world state \" + (::RootState.get(\"WorldState\") != null) + \", loading \" + ::LoadingScreen.isVisible() + \", world visible \" + ::World.isVisible() + \", menu \" + (::RootState.get(\"MainMenuState\") != null); })()",
				)
				.ok()
				.flatten()
				.unwrap_or_default();
				return Err(format!(
					"{what}: not shown after {MAX_FRAMES} frames (holds {}, last frame {} quads, {} waiting; {state})",
					self.holds(cond),
					stats.quads,
					stats.waiting
				));
			}
		}
	}
}

fn report(what: &str, s: &Shown) {
	let screen = match (s.screen_up, s.screen_gone) {
		(Some(up), Some(gone)) => format!("; loading screen up at {up:.2} s, gone at {gone:.2} s"),
		(Some(up), None) => format!("; loading screen up at {up:.2} s"),
		_ => String::new(),
	};
	println!(
		"{what}: {:.2} s ({} frames), pop-in: {} frames, at most {} quads waiting{screen}",
		s.seconds, s.frames, s.pop_in_frames, s.most_waiting
	);
}

fn menu(start: Instant) -> Result<(), String> {
	let gpu = Gpu::headless_with(bb_app::front::GPU_FEATURES)?;
	let config = bb_ui::host::HostConfig {
		exact_clock: squirrel_bb::ExactClock::Live,
		sink: Some(bb_app::front::gpu_page::sink(bb_render::UiPage::new(&gpu)?)),
		..bb_ui::host::HostConfig::default()
	};
	let session = bb_ui::host::UiSession::start(Path::new(STEAM_DATA_DIR), config, bb_ui::host::Echo::Off)?;
	session.boot_menu()?;
	let at_menu = || player::eval_i64(session.engine(), &format!("({AT_MENU}) ? 1 : 0")).ok().flatten() == Some(1);
	let mut frames = 0;
	while !at_menu() {
		session.step(STEP_MS)?;
		frames += 1;
		if frames > MAX_FRAMES {
			return Err("the main menu never showed".into());
		}
	}
	println!("boot to the main menu: {:.2} s ({frames} frames)", start.elapsed().as_secs_f64());
	Ok(())
}

fn game(start: Instant) -> Result<(), String> {
	let gpu = Gpu::headless_with(bb_app::front::GPU_FEATURES)?;
	let game = Game::boot(&gpu, Path::new(STEAM_DATA_DIR), "rust-engine-test", false, None, VideoMode::DEFAULT, None)?;
	let (width, height) = game.screen();
	let target = RenderTarget::new(&gpu, width, height);
	let mut run = Run {
		game,
		gpu: &gpu,
		target,
		spent: Default::default(),
	};
	harness::install_autoplay(run.game.front().engine())?;
	report("start to the first campaign's world", &run.until_shown(start, "the first world", IN_WORLD)?);
	let (step, draw, checks) = run.spent.get();
	println!("  of which the game's steps {step:.2} s, its draws {draw:.2} s, this tool's checks {checks:.2} s");

	// A new campaign from the main menu, after quitting to it from the world.
	run.game.front().engine().run_script("::World.State.exitGame();", "quit to the menu")?;
	let mut frames = 0;
	while !run.holds(AT_MENU) {
		run.frame()?;
		frames += 1;
		if frames > MAX_FRAMES {
			return Err("the main menu never came back".into());
		}
	}
	// Start once the last loading screen has faded out, and a moment after, in real time
	// (the page's animations run on its own clock). A show within ~400 ms of a hide is
	// lost, in the original too (probed).
	let mut frames = 0;
	while run.holds("::LoadingScreen.isVisible() || ::LoadingScreen.isAnimating()") {
		run.frame()?;
		frames += 1;
		if frames > MAX_FRAMES {
			return Err("the menu's loading screen never faded out".into());
		}
	}
	let settle = Instant::now();
	while settle.elapsed().as_secs_f64() < 0.3 {
		run.frame()?;
		std::thread::sleep(std::time::Duration::from_millis(16));
	}
	let t = Instant::now();
	run.game.front().session().start_campaign("rust-engine-test-2")?;
	report("a new campaign from the main menu", &run.until_shown(t, "the second world", IN_WORLD)?);

	// A load: a save of this world, loaded again (the world state ignores a load within
	// 4 s of the last).
	run.game.front().engine().run_script("::World.State.saveCampaign(\"bb_probe_load_times\");", "save")?;
	for _ in 0..300 {
		run.frame()?;
	}
	let t = Instant::now();
	// As the in-game menu's Load does: behind the loading screen.
	run.game.front().engine().run_script(
		"::World.State.setCampaignToLoadFileName(\"bb_probe_load_times\"); ::LoadingScreen.show();",
		"load",
	)?;
	// The loading screen shows as its page animates it; the load happens once it's shown.
	let mut frames = 0;
	while !run.holds("::LoadingScreen.isVisible()") {
		run.frame()?;
		frames += 1;
		if frames > MAX_FRAMES {
			return Err("the load's loading screen never showed".into());
		}
	}
	report("a load", &run.until_shown(t, "the loaded world", IN_WORLD)?);

	// A battle's start.
	for _ in 0..120 {
		run.frame()?;
	}
	run.game.front().engine().run_script("::World.State.setAutoPause(false); ::World.State.setPause(false);", "unpause")?;
	let t = Instant::now();
	run.game.front().engine().run_script(ENGAGE, "engage")?;
	report("a battle's start", &run.until_shown(t, "the battle", IN_BATTLE)?);
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	let start = Instant::now();
	bb_app::platform::tune_allocator();
	let phase = std::env::args().nth(1).unwrap_or_default();
	let result = match phase.as_str() {
		"menu" => menu(start),
		"game" => game(start),
		_ => Err("load_times menu|game".to_string()),
	};
	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		}
	}
}
