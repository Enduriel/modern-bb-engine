//! The frame parity scenes (`parity/frames/scenes.{nut,js}`), drawn offscreen as `bb`
//! draws them: the original's are screenshots a probe takes, ours come from here, for the
//! `frame_scenes` example (diffed with the original's) and the `frame_regression` test
//! (diffed with ours from before).
//!
//! The campaign is the probe's (seed `bbuitest`), and its scene script runs where the
//! probe runs it (right after `world_state.init()`). Once the intro event shows, the
//! script's `::probeSteps` are walked as the probe's page walks them: each step's scene
//! set up, its `until` polled, 3 s to settle, its shot taken, its button clicked (and
//! checked to have done what its `clicked` says). The drawn cursor is left out: the
//! original's screenshots have none.
//!
//! Frames are drawn as `bb` draws them: one that would show what the last did is left
//! undrawn ([`Game::draw_if_changed`]). So each shot is the frame left up, and is checked
//! to be pixel for pixel what drawing it again draws.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bb_assets::Image;
use bb_engine::harness::STEAM_DATA_DIR;
use bb_render::{Gpu, RenderTarget};
use squirrel::SqTable;
use squirrel_bb::VideoMode;

use crate::game::Game;

const SEED: &str = "bbuitest";
const STEP: Duration = Duration::from_secs(180);

/// The frame parity scenes' script, the probe's own.
pub fn default_scenes() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../parity/frames/scenes.nut")
}

/// One of the script's `::probeSteps`.
struct Step {
	scene: Option<String>,
	shot: Option<String>,
	until: Option<String>,
	click: Option<String>,
	/// What its click brings about: a JS expression on the page that holds once it has.
	clicked: Option<String>,
	/// Its `until` is polled without pacing frames to the clock (`unpaced = true`).
	unpaced: bool,
	/// Starts the clock of the timed steps that follow (`start = true`): no settling
	/// after its scene, and no shot.
	start: bool,
	/// A timed step (`at`, ms): its scene and shot that long after the last `start`
	/// step's scene, without settling.
	at: Option<f64>,
}

/// The script's `::probeSteps`.
fn steps(game: &Game) -> Result<Vec<Step>, String> {
	let engine = game.front().engine();
	engine.run_script(
		r#"::__steps <- "";
		foreach (s in ::probeSteps)
		{
			foreach (k in ["scene", "shot", "until", "click", "clicked", "at"])
				::__steps += (!(k in s) || s[k] == null ? "" : s[k]) + "\t";
			::__steps += ("unpaced" in s && s.unpaced ? "1" : "") + "\t";
			::__steps += ("start" in s && s.start ? "1" : "") + "\n";
		}"#,
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
				clicked: some(f[4]),
				at: f[5].parse().ok(),
				unpaced: f[6] == "1",
				start: f[7] == "1",
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

/// Run `seconds` of frames at `hz` a second, as the game loop does, drawing each that
/// shows something new; `paced`, each takes at least its share of a second of the clock
/// too (the page's animations run on it), else they come as fast as they're drawn. How
/// many were left undrawn.
fn play(game: &mut Game, gpu: &Gpu, target: &RenderTarget, seconds: f64, hz: f64, paced: bool) -> Result<u32, String> {
	let frame = Duration::from_secs_f64(1.0 / hz);
	let mut undrawn = 0;
	for _ in 0..(seconds * hz).round() as u32 {
		let start = Instant::now();
		game.step(1000.0 / hz)?;
		if game.draw_if_changed(gpu, target).is_none() {
			undrawn += 1;
		}
		if let Some(rest) = frame.checked_sub(start.elapsed()).filter(|_| paced) {
			game.front().session().host().idle(rest);
		}
	}
	Ok(undrawn)
}

/// Click the button `label` once it's ready and give the game 3 s, as the probe's page
/// does; then `done` (a JS expression) must hold: the page seeing the click doesn't say
/// its handler ran. A click that didn't take is logged with what it hit and made again,
/// once.
fn click(game: &mut Game, gpu: &Gpu, target: &RenderTarget, label: &str, done: &str) -> Result<(), String> {
	let mut first = None;
	loop {
		let session = game.front().session();
		let (x, y) = session.locate_when_ready(label, STEP).map_err(|e| match &first {
			Some(trail) => format!("clicking {label:?} didn't take ({done} still false; {trail}), and clicking it again: {e}"),
			None => e,
		})?;
		session.click_at(x, y)?;
		session.move_away()?;
		play(game, gpu, target, 3.0, 60.0, true)?;
		let session = game.front().session();
		if session.holds(done)? {
			return Ok(());
		}
		let trail = session.click_trail(x, y);
		match first {
			None => {
				println!("clicking {label:?} didn't take ({done} still false after 3 s): {trail}; clicking again");
				first = Some(trail);
			}
			Some(first) => {
				return Err(format!("clicking {label:?} didn't take, twice ({done} still false); first: {first}; again: {trail}"));
			}
		}
	}
}

/// What `target` holds (the frame left up), checked against the frame drawn again.
fn shot_checked(game: &mut Game, gpu: &Gpu, target: &RenderTarget, name: &str) -> Result<Image, String> {
	let left = target.read(gpu)?;
	game.draw(gpu, target);
	let drawn = target.read(gpu)?;
	let differ = left.rgba.chunks(4).zip(drawn.rgba.chunks(4)).filter(|(a, b)| a != b).count();
	if differ > 0 {
		return Err(format!("{name}: the frame left up differs from the one drawn again in {differ} pixels"));
	}
	Ok(left)
}

/// Walk the scenes on a `screen` of the given size (the original's video mode: a probe
/// can't change it), handing each shot to `shot` by name as it's taken.
pub fn shoot(gpu: &Gpu, screen: (i64, i64), shot: impl FnMut(&str, Image) -> Result<(), String>) -> Result<(), String> {
	shoot_scenes(gpu, screen, &default_scenes(), shot)
}

/// As [`shoot`], walking the steps of another scene script (a probe's own, written as
/// `parity/frames/scenes.nut` is).
pub fn shoot_scenes(
	gpu: &Gpu,
	screen: (i64, i64),
	scenes: &Path,
	mut shot: impl FnMut(&str, Image) -> Result<(), String>,
) -> Result<(), String> {
	let script = std::fs::read_to_string(scenes).map_err(|e| format!("{}: {e}", scenes.display()))?;
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
	let mut game = Game::boot(gpu, Path::new(STEAM_DATA_DIR), SEED, false, None, mode, None)?;
	{
		let session = game.front().session();
		let engine = session.engine();
		// Where the probe harness runs it: right after world_state.init(), before the
		// world's first update (see bb-ui's goldens::capture_world).
		let deadline = Instant::now() + STEP;
		loop {
			engine.init_pending_states()?;
			if engine.root_state().state("WorldState").is_some() {
				engine.run_script(&script, "scenes.nut")?;
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
	let target = RenderTarget::new(gpu, width, height);
	// Game seconds since the last `start` step's scene.
	let mut clock = 0.0;
	for step in steps(&game)? {
		if let Some(at) = step.at {
			let frames = ((at / 1000.0 - clock) * 60.0).round().max(0.0);
			let undrawn = play(&mut game, gpu, &target, frames / 60.0, 60.0, true)?;
			clock += frames / 60.0;
			if let Some(name) = &step.scene {
				scene(&game, name)?;
			}
			if let Some(name) = &step.shot {
				println!("{name}: at {clock:.3} s, {undrawn} of {frames} frames left undrawn before the shot");
				shot(name, shot_checked(&mut game, gpu, &target, name)?)?;
			}
			continue;
		}
		if let Some(name) = &step.scene {
			scene(&game, name)?;
		}
		if step.start {
			clock = 0.0;
			continue;
		}
		if let Some(until) = &step.until {
			// Counted in the game's own time, not the clock's: frames come slower on a
			// busy machine, and each still advances the game by a 60th of a second.
			let mut waited = 0.0;
			while !scene(&game, until)? {
				if waited >= STEP.as_secs_f64() {
					return Err(format!("{until} never came true in {waited} s of game time"));
				}
				play(&mut game, gpu, &target, 0.5, 60.0, !step.unpaced)?;
				waited += 0.5;
			}
		}
		// A battle's last second at 180 frames a second, as on a 180 Hz display: its
		// animations step at 60 a second (see `squirrel_bb::scene::steps`), so the frame
		// left up for the shot is most likely one drawn a frame or two before.
		let battle = game.in_battle();
		let (hz, frames) = if battle { (180.0, 300) } else { (60.0, 180) };
		let mut undrawn = play(&mut game, gpu, &target, if battle { 2.0 } else { 3.0 }, 60.0, true)?;
		if battle {
			undrawn += play(&mut game, gpu, &target, 1.0, hz, true)?;
		}
		if let Some(name) = &step.shot {
			println!("{name}: {undrawn} of {frames} frames left undrawn before the shot");
			shot(name, shot_checked(&mut game, gpu, &target, name)?)?;
		}
		if let Some(label) = &step.click {
			let done = step.clicked.as_deref().ok_or(format!("the step clicking {label:?} names no `clicked`"))?;
			click(&mut game, gpu, &target, label, done)?;
		}
	}
	Ok(())
}
