//! The pointer moving over the game while a campaign starts, with the install's mods:
//! every frame of the start, from the main menu's Start through MSU's
//! new-campaign settings, the loading screen and worldgen to the world with its opening
//! event, the cursor moves, as a player's does. No script errs. Configurable Pause's
//! hook on `world_state.onMouseInput` reads `this.getPlayer().m` before vanilla's own
//! loading-screen check, so it errs on a mouse event that reaches the world state
//! before its `onInit` (which spawns the player's party). The original gives an added
//! state no input before its `onInit` (probed: none of a steady stream of pointer moves
//! reaches it between `RootState.add` and its `onInit`); nor does ours.
//!
//! The FPS display is on from the start, as `bb --mods --fps` has it: it shows the frame
//! rate on the main menu and over the world, though Modern Hooks replaces the page's
//! root screen, and the game's `setFPS` never reaches a module without its label.
//!
//! Skips without a Steam install, without the mods this needs (Modern Hooks, MSU,
//! Configurable Pause) or without a GPU; fails if the install's mods are not the pinned
//! corpus (`bb_engine::mod_corpus`).
//!
//! Run: `cargo test -p bb-app --features ui --test mods_start_input`
//!
//! A plain `main` (`harness = false`): the browser host re-runs this binary for its
//! child processes, and starts only once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_app::game::{Boot, Game, Start};
use bb_engine::harness::STEAM_DATA_DIR;
use bb_engine::mod_corpus::CorpusError;
use bb_render::Gpu;
use squirrel::SqTable;
use squirrel_bb::VideoMode;

const NAME: &str = "the_pointer_moves_while_a_modded_campaign_starts";
/// The mods' zips (name prefixes) this needs.
const MODS: [&str; 3] = ["mod_modern_hooks", "mod_msu", "mod_configurable_pause"];

/// The values the FPS displays on the page show (hidden ones as `(hidden)`).
fn fps_shown(game: &Game) -> Result<Vec<String>, String> {
	let shown = game.front().session().eval(
		"(function () {
			var all = document.querySelectorAll('.fps-container');
			var out = [];
			for (var i = 0; i < all.length; i++) {
				var v = all[i].querySelector('.value');
				out.push(all[i].closest('.root-screen') !== null && all[i].offsetParent !== null && v ? v.textContent : '(hidden)');
			}
			return out;
		})()",
	)?;
	Ok(shown.as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

/// The FPS display shows one frame rate within a second of frames (its first value
/// comes after a quarter second), and the page has logged no error over it.
fn fps_shows(game: &mut Game, when: &str) -> Result<(), String> {
	let start = Instant::now();
	loop {
		let errors: Vec<String> = game
			.front()
			.session()
			.log()
			.into_iter()
			.filter(|line| (line.level == "error" || line.level == "bridge") && line.text.contains("FPS"))
			.map(|line| line.text)
			.collect();
		if !errors.is_empty() {
			return Err(format!("{when}: the FPS display: page errors: {errors:?}"));
		}
		match fps_shown(game)?.as_slice() {
			[value] if value.parse::<u32>().is_ok_and(|fps| fps > 0) => return Ok(()),
			other if start.elapsed() > Duration::from_secs(1) => {
				return Err(format!("{when}: the FPS displays show {other:?}, not one frame rate"));
			}
			_ => {}
		}
		game.step(1000.0 / 60.0)?;
		// As a window presents each frame (the FPS display counts them).
		game.presented();
		std::thread::sleep(Duration::from_millis(16));
	}
}

/// A Squirrel condition, evaluated in the scripts.
fn holds(game: &Game, expr: &str) -> Result<bool, String> {
	game.run_script(&format!("::__holds <- ({expr}) ? true : false;"))?;
	SqTable::root(game.front().engine().vm())
		.get::<bool>("__holds")
		.ok_or_else(|| "no answer".to_string())
}

fn check(gpu: &Gpu) -> Result<(), String> {
	let boot = Boot {
		data_dir: Path::new(STEAM_DATA_DIR),
		start: Start::Menu,
		mods: true,
		sound: false,
		world_size: None,
		mode: VideoMode::DEFAULT,
		settings: None,
		devtools: None,
		saves: None,
		cef_log: None,
		store: Default::default(),
		update_check: None,
		display: None,
	};
	// The saves (and MSU's storages) stay in memory: the player's are never touched.
	let mut game = Game::start(gpu, &boot)?;
	// As `bb --fps` does, once the game has started.
	game.show_fps(true)?;
	let (width, height) = game.screen();
	let (x, y) = (i64::from(width / 2), i64::from(height / 2));
	let mut frame = 0i64;
	// The main menu, a few frames, the pointer moving (MSU's late JS has connected its
	// screens by the time the boot returns).
	for _ in 0..30 {
		game.mouse_moved(x + frame % 7, y).map_err(|e| format!("frame {frame}: a pointer move erred: {e}"))?;
		game.step(1000.0 / 60.0)?;
		frame += 1;
	}
	fps_shows(&mut game, "on the main menu")?;
	// The menu's Start, as a player's click on it (the probe harness's campaign settings).
	game.front().session().start_campaign("bbuitest")?;
	let mut pressed_save = false;
	let deadline = Instant::now() + Duration::from_secs(300);
	loop {
		if Instant::now() >= deadline {
			return Err(format!("the world never came up ({frame} frames)"));
		}
		// The cursor wanders over the middle of the screen every frame, as a hand on the
		// mouse does while the game loads.
		let dx = frame % 7 * 3;
		game.mouse_moved(x + dx, y - dx).map_err(|e| format!("frame {frame}: a pointer move erred: {e}"))?;
		game.step(1000.0 / 60.0).map_err(|e| format!("frame {frame}: {e}"))?;
		frame += 1;
		// MSU's new-campaign settings page: its Save goes on with the start.
		if !pressed_save && holds(&game, "::MSU.SettingsScreen.isVisible()")? {
			game.run_script("::MSU.SettingsScreen.onSaveButtonPressed({});")?;
			pressed_save = true;
		}
		let up = "\"State\" in ::World && ::World.State != null && ::World.State.getPlayer() != null \
			&& !::World.State.isInLoadingScreen()";
		if holds(&game, up)? {
			break;
		}
	}
	if !pressed_save {
		return Err("MSU's new-campaign settings page never showed".into());
	}
	// On over the world for a moment, where the world state takes the moves.
	for _ in 0..30 {
		game.mouse_moved(x + frame % 5, y).map_err(|e| format!("over the world: {e}"))?;
		game.step(1000.0 / 60.0)?;
		frame += 1;
	}
	fps_shows(&mut game, "over the world")?;
	println!("  {frame} frames, the cursor moving in each");
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// A runner's `--list` (cargo-nextest's), answered before a display starts.
	if let Some(code) = bb_engine::harness::list_case(NAME) {
		return code;
	}
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	let data = Path::new(STEAM_DATA_DIR);
	if !data.exists() {
		return bb_engine::harness::skip_case(NAME, format!("{STEAM_DATA_DIR} not present"));
	}
	let mods = match bb_engine::mod_corpus::installed(data) {
		Ok(mods) => mods,
		Err(CorpusError::Absent(why)) => return bb_engine::harness::skip_case(NAME, why),
		Err(e) => {
			println!("test {NAME} ... FAILED\n{e}");
			return ExitCode::FAILURE;
		}
	};
	let missing: Vec<&str> = MODS
		.iter()
		.copied()
		.filter(|m| !mods.iter().any(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(m))))
		.collect();
	if !missing.is_empty() {
		return bb_engine::harness::skip_case(NAME, format!("mods not installed: {missing:?}"));
	}
	let gpu = match Gpu::headless_with(bb_app::front::GPU_FEATURES) {
		Ok(gpu) => gpu,
		Err(e) => {
			return bb_engine::harness::skip_case(NAME, format!("no GPU: {e}"));
		}
	};
	match check(&gpu) {
		Ok(()) => {
			println!("test {NAME} ... ok");
			ExitCode::SUCCESS
		}
		Err(e) => {
			println!("test {NAME} ... FAILED\n{e}");
			ExitCode::FAILURE
		}
	}
}
