//! The music's and the ambience's timers are `Time` events, as in the original
//! (disassembled and probed 2026-10-20): the ambience's next sound (each layer's) and a
//! silent music's retry every 3 s are Real events on the scripts' list. So:
//! - `Time.hasEventScheduled(TimeUnit.Real)` and `(4)` are true while the world's
//!   ambience runs (the original answers `10001` for units 0–4);
//! - `Time.clearEvents()` cancels them, and nothing re-arms them by itself: the ambience
//!   falls silent until a script sets a layer again, and a silent music no longer
//!   retries (the original stayed at `00000` for 7 s, and 4 s);
//! - the states' clears (a load, a battle's end, quitting to the menu) drop them. After
//!   a load the world's next update sets its ambience again (its `LastPlayerTile` was
//!   reset). After a battle it stays silent until the company moves to another tile (or
//!   day turns to night): the world sets it on the battle's "AboutToFinish", and the
//!   battle's state finishes after that, in the next update pass (`onFinish`: the clear
//!   and `stopAmbience`).
//!
//! Silent (the harness's output keeps time but plays nothing). Skips without the Steam
//! install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
const AT_MENU: &str = "::RootState.get(\"WorldState\") == null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible() && !::RootState.get(\"MainMenuState\").m.MainMenuScreen.isAnimating()";
const LOADED: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";
const TACTICAL: &str = "TacticalFromWorldState";
/// The test's save, kept in memory (the engine has no save directory here).
const SAVE: &str = "bb_probe_audio_timers";

/// `hasEventScheduled` for units 0–4 as a string of bits (`::__has()`), and every
/// `clearEvents()` call logged with it before and after (`::__clears`).
const INSTRUMENT: &str = r#"
::__has <- function()
{
	local s = "";
	for (local u = 0; u < 5; u++) s += ::Time.hasEventScheduled(u) ? "1" : "0";
	return s;
};
::__clears <- [];
local clear = ::Time.clearEvents;
::Time.clearEvents = function()
{
	local before = ::__has();
	clear();
	::__clears.push(before + ">" + ::__has());
};
"#;

fn frame(engine: &harness::GameEngine) {
	harness::run_frame(engine, 1000.0 / 60.0).unwrap();
}

fn frames(engine: &harness::GameEngine, n: u32) {
	for _ in 0..n {
		frame(engine);
	}
}

fn until(engine: &harness::GameEngine, max: u32, what: &str, cond: &str) {
	for _ in 0..max {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap() == Some(1) {
			return;
		}
		frame(engine);
	}
	panic!("{what}: still not true after {max} frames: {cond}");
}

fn has(engine: &harness::GameEngine) -> String {
	player::eval_string(engine, "::__has()").unwrap().unwrap()
}

fn run(engine: &harness::GameEngine, code: &str) {
	engine.run_script(code, "audio timers").unwrap();
}

fn music_playing(engine: &harness::GameEngine) -> bool {
	player::eval_i64(engine, "::Music.isPlaying() ? 1 : 0").unwrap() == Some(1)
}

/// The clears logged since the last call.
fn clears(engine: &harness::GameEngine) -> String {
	let s = player::eval_string(engine, "::__clears.len() ? ::__clears.reduce(@(a, b) a + \",\" + b) : \"\"").unwrap().unwrap_or_default();
	run(engine, "::__clears.clear();");
	s
}

#[test]
fn the_audio_timers_are_time_events() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let mut engine = harness::open_install(dir).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	harness::install_autoplay(&engine).expect("autoplay");
	until(&engine, 600, "the main menu", AT_MENU);
	run(&engine, INSTRUMENT);
	assert_eq!(has(&engine), "00000", "the menu's music plays: no timer");

	// A campaign, past its origin's intro.
	run(&engine, &format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"));
	frame(&engine);
	run(
		&engine,
		&format!(
			"{MENU}.getNewCampaignMenuModule().onStartButtonPressed({{
				companyName = \"Timers\", banner = \"banner_04\",
				combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
				ironman = false, explorationDifficulty = false,
				lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
				seed = \"rust-engine-test\", origin = \"scenario.early_access\"
			}});"
		),
	);
	until(&engine, 600, "the campaign loading", LOADED);
	harness::run_past_intro(&engine).expect("past the intro");
	frames(&engine, 60);
	assert!(!engine.time().script_event_scheduled(4), "no script's event pending");
	assert_eq!(has(&engine), "10001", "the ambience's timers: Real events");

	// A clear cancels them, and nothing re-arms them by itself.
	run(&engine, "::Time.clearEvents();");
	assert_eq!(has(&engine), "00000");
	frames(&engine, 7 * 60);
	assert_eq!(has(&engine), "00000", "the ambience stays silent");
	assert!(music_playing(&engine), "the music plays on");
	// Until the world sets its ambience again.
	run(&engine, "::World.State.m.LastPlayerTile = null;");
	frame(&engine);
	assert_eq!(has(&engine), "10001", "set again: its timers again");

	// A silent music retries every 3 s, until a clear.
	run(&engine, "::Time.clearEvents(); ::Sound.stopAmbience(); ::__vol <- ::Settings.getSoundVolume(::Const.Sound.Channel.Music); ::Settings.setSoundVolume(::Const.Sound.Channel.Music, 0.0);");
	assert!(!music_playing(&engine), "volume 0: paused");
	assert_eq!(has(&engine), "00000");
	run(&engine, "::Music.setTrackList(::Const.Music.WorldmapTracks, ::Const.Music.CrossFadeTime);");
	assert_eq!(has(&engine), "10001", "the silent music's retry");
	frames(&engine, 4 * 60);
	assert_eq!(has(&engine), "10001", "retried, and again");
	run(&engine, "::Time.clearEvents();");
	frames(&engine, 4 * 60);
	assert_eq!(has(&engine), "00000", "no retry after the clear");
	run(&engine, "::Settings.setSoundVolume(::Const.Sound.Channel.Music, ::__vol);");
	assert!(music_playing(&engine), "the held track plays again");
	assert_eq!(has(&engine), "00000");
	clears(&engine);

	// A load drops them; the world's next update sets the ambience again.
	run(&engine, "::World.State.m.LastPlayerTile = null;");
	frame(&engine);
	assert_eq!(has(&engine), "10001");
	run(&engine, &format!("::World.State.saveCampaign(\"{SAVE}\");"));
	run(&engine, &format!("::World.State.m.CampaignLoadTime = -100.0; ::World.State.loadCampaign(\"{SAVE}\");"));
	assert_eq!(clears(&engine), "10001>00000", "the load's clear");
	assert_eq!(has(&engine), "00000");
	assert!(player::eval_i64(&engine, "::World.State.m.LastPlayerTile == null ? 1 : 0").unwrap() == Some(1));
	frame(&engine);
	assert_eq!(has(&engine), "10001", "the world's ambience again");

	// A battle: its ambience runs on timers too; its end drops them, and the world
	// sets its own again.
	run(&engine, "::__bootWorldState <- ::World.State;");
	harness::unpause_world(&engine).expect("unpause");
	harness::engage_nearest_hostile(&engine).expect("engage");
	until(&engine, 600, "the battle", &format!("::RootState.get(\"{TACTICAL}\") != null"));
	frames(&engine, 60);
	assert!(has(&engine).starts_with('1'), "the battle's ambience: {}", has(&engine));
	clears(&engine);
	let mut fought = true;
	for _ in 0..30_000 {
		frame(&engine);
		if !engine.root_state().state_names().iter().any(|s| s == TACTICAL) {
			fought = false;
			break;
		}
	}
	assert!(!fought, "the battle never ended");
	let at_end = clears(&engine);
	assert!(at_end.split(',').any(|c| c.starts_with('1') && c.ends_with(">00000")), "the battle's end cleared its timers: {at_end}");
	// The world set its ambience on the battle's "AboutToFinish" (`onCombatFinished`),
	// before the battle's state finished in the next update pass and cleared it (and
	// stopped the ambience): it stays silent until the company moves on.
	frames(&engine, 10 * 60);
	assert_eq!(has(&engine), "00000", "silent after the battle");
	run(&engine, "::World.State.m.LastPlayerTile = null;");
	frame(&engine);
	assert!(has(&engine).starts_with('1'), "until the world sets it again: {}", has(&engine));

	// Quitting to the menu drops them.
	run(&engine, "::World.State.exitGame();");
	until(&engine, 600, "back at the menu", AT_MENU);
	let quit = clears(&engine);
	assert!(quit.split(',').any(|c| c.starts_with('1') && c.ends_with(">00000")), "the quit's clear: {quit}");
	frames(&engine, 10 * 60);
	assert_eq!(has(&engine), "00000", "nothing at the menu");
}
