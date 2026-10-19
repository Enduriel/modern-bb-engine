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
//! Skips without a Steam install, without the mods this needs (Modern Hooks, MSU,
//! Configurable Pause) or without a GPU.
//!
//! Run: `cargo test -p bb-app --features ui --test mods_start_input`
//!
//! A plain `main` (`harness = false`): the browser host re-runs this binary for its
//! child processes, and starts only once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_app::game::{Boot, Game, Start};
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::Gpu;
use squirrel::SqTable;
use squirrel_bb::VideoMode;

const NAME: &str = "the_pointer_moves_while_a_modded_campaign_starts";
/// The mods' zips (name prefixes) this needs.
const MODS: [&str; 3] = ["mod_modern_hooks", "mod_msu", "mod_configurable_pause"];

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
		devtools: None,
		saves: None,
		store: Default::default(),
	};
	// The saves (and MSU's storages) stay in memory: the player's are never touched.
	let mut game = Game::start(gpu, &boot)?;
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
	println!("  {frame} frames, the cursor moving in each");
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let data = Path::new(STEAM_DATA_DIR);
	if !data.exists() {
		println!("test {NAME} ... skipped ({STEAM_DATA_DIR} not present)");
		return ExitCode::SUCCESS;
	}
	let mods = harness::mod_archives(data).unwrap_or_default();
	let missing: Vec<&str> = MODS
		.iter()
		.copied()
		.filter(|m| !mods.iter().any(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(m))))
		.collect();
	if !missing.is_empty() {
		println!("test {NAME} ... skipped (mods not installed: {missing:?})");
		return ExitCode::SUCCESS;
	}
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			println!("test {NAME} ... skipped (no GPU: {e})");
			return ExitCode::SUCCESS;
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
