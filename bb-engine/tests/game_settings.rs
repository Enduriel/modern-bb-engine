//! The scripts' settings besides the video mode, as the original keeps them
//! (disassembled 2026-10-20, see `squirrel_bb`'s `native::settings`):
//! - `getControlSettings()` is the game's one: writes stick (edge-of-screen scrolling
//!   can be turned on, the Options screen shows what was set);
//! - `setHardwareSound` is kept;
//! - `Settings.save()` hands the host every setting the original's config file holds
//!   (gameplay, controls, volumes, hardware sound) with the video mode;
//! - `Settings.load(false)` reads back what the host's file holds;
//! - the host's settings start the scripts' (`Engine::apply_game_settings`).
//!
//! Skips cleanly when the Steam install isn't there.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};
use squirrel::SqTable;
use squirrel_bb::GameSettings;

fn eval(engine: &GameEngine, expr: &str) -> String {
	engine.run_script(&format!("::__v <- ({expr}).tostring();"), "eval").unwrap_or_else(|e| panic!("{expr}: {e}"));
	SqTable::root(engine.vm()).get::<String>("__v").expect(expr)
}

fn run(engine: &GameEngine, code: &str) {
	engine.run_script(code, "settings").unwrap_or_else(|e| panic!("{code}: {e}"));
}

#[test]
fn the_settings_stick_save_and_load() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	let video = engine.video();
	assert_eq!(engine.game_settings(), harness::PROBED_INSTALL_SETTINGS, "the harness's settings");

	// The control settings are the game's own: a write through one shows in the next.
	run(&engine, "local c = ::Settings.getControlSettings(); c.UseDragStyleScrolling = false; c.UseHardwareCursor = true;");
	assert_eq!(eval(&engine, "::Settings.getControlSettings().UseDragStyleScrolling"), "false");
	assert_eq!(eval(&engine, "::Settings.getControlSettings().UseHardwareCursor"), "true");

	// Hardware sound is kept; its argument converts as a bool parameter does.
	run(&engine, "::Settings.setHardwareSound(false);");
	assert_eq!(eval(&engine, "::Settings.isHardwareSound()"), "false");
	run(&engine, "::Settings.setHardwareSound(1);");
	assert_eq!(eval(&engine, "::Settings.isHardwareSound()"), "true");
	run(&engine, "::Settings.setHardwareSound(0);");

	// save(): everything, as the Options screen's helpers leave it.
	run(
		&engine,
		"::Settings.getGameplaySettings().AutoLoot = true;
		::Settings.getGameplaySettings().AlwaysHideTrees = false;
		::Settings.setSoundVolume(-1, 0.25);
		::Settings.setSoundVolume(0, 0.5);
		::Settings.save();",
	);
	let saved = video.take_save().expect("settings to keep");
	let game = saved.game;
	assert_eq!(game.gameplay("AutoLoot"), Some(true));
	assert_eq!(game.gameplay("AlwaysHideTrees"), Some(false));
	assert_eq!(game.gameplay("DontAutoEndTurns"), Some(true), "as it was");
	assert!(!game.drag_scrolling && game.hardware_cursor && !game.hardware_sound, "{game:?}");
	assert_eq!(game.volumes, [0.25, 0.5, 0.7, 0.4]);
	assert_eq!(saved.video, video.mode());

	// load(false): back to what the file holds, unsaved changes gone.
	run(&engine, "::Settings.getGameplaySettings().AutoLoot = false; ::Settings.getControlSettings().UseDragStyleScrolling = true; ::Settings.setSoundVolume(-1, 1.0);");
	assert_eq!(eval(&engine, "::Settings.load(false)"), "true");
	assert_eq!(engine.game_settings(), game);

	// The host's settings: a fresh install's.
	engine.apply_game_settings(&GameSettings::FRESH_INSTALL).expect("apply");
	assert_eq!(engine.game_settings(), GameSettings::FRESH_INSTALL);
	assert_eq!(eval(&engine, "::Settings.getGameplaySettings().ShowOrientationOverlays"), "true");
	assert_eq!(eval(&engine, "::Settings.getGameplaySettings().DontAutoEndTurns"), "false");
	assert_eq!(eval(&engine, "::Settings.getSoundVolume(0)"), "0.7");
	assert_eq!(eval(&engine, "::Settings.getControlSettings().UseDragStyleScrolling"), "true");
}
