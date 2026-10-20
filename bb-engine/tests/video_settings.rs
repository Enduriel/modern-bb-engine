//! The scripts' video mode, as the original's (see `squirrel_bb::native::video_mode`):
//! `Settings.getVideoMode()` is the game's own mode, so a write to it shows at once;
//! `setVideoMode` tells the host and sets the frame cap from VSync; the save natives
//! hand the host the mode to keep (with the other settings, see `game_settings.rs`); the resolution queries hand out copies. Skips
//! cleanly when the Steam install isn't there.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};
use squirrel::SqTable;

fn eval_f64(engine: &GameEngine, expr: &str) -> f64 {
	engine.run_script(&format!("::__v <- ({expr}).tofloat();"), "video").expect(expr);
	SqTable::root(engine.vm()).get::<f64>("__v").expect(expr)
}

#[test]
fn the_scripts_video_mode_is_the_games() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	let video = engine.video();

	// A write to the mode shows at once, to the scripts and the host, but nothing is
	// applied until setVideoMode.
	engine
		.run_script("local m = ::Settings.getVideoMode(); m.UIScale = 1.5;", "write")
		.expect("write");
	assert_eq!(eval_f64(&engine, "::Settings.getVideoMode().UIScale"), 1.5);
	assert_eq!(video.mode().ui_scale, 1.5);
	assert_eq!(video.changes(), 0);

	// setVideoMode: applied, and VSync off caps the frame rate at 144.
	engine
		.run_script("local m = ::Settings.getVideoMode(); m.VSync = false; ::Settings.setVideoMode(m);", "set")
		.expect("set");
	assert_eq!(video.changes(), 1);
	let mode = video.mode();
	assert!(mode.limit_frame_rate && mode.frame_cap == 144, "{mode:?}");
	assert_eq!(eval_f64(&engine, "::Settings.getVideoMode().LimitFrameRate ? 1 : 0"), 1.0);

	// A queried resolution is a copy: writing it changes nothing.
	engine
		.run_script(
			"local r = ::Settings.queryResolutionByIndex(0, 1024, 768, 32); if (r != null) r.Width = 7;",
			"copy",
		)
		.expect("copy");
	assert_ne!(video.mode().width, 7);

	// The save natives hand the host the mode to keep, once.
	assert_eq!(video.take_save(), None);
	engine
		.run_script(
			"local m = ::Settings.getVideoMode(); m.Width = 1280; m.Height = 720; ::Settings.saveWithCustomVideoMode(m);",
			"save",
		)
		.expect("save");
	let saved = video.take_save().expect("a mode to keep");
	assert_eq!((saved.video.width, saved.video.height), (1280, 720));
	assert_eq!(video.take_save(), None);
}
