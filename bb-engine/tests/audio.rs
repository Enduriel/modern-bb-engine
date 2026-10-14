//! The game's sound as the scripts drive it, silent (the harness's output keeps time
//! but plays nothing): the menu's music, the volume settings, effects, and the world's
//! ambience and music. Skips cleanly when the Steam install isn't there.
//!
//! The clock is the frames' (each 1/60 s), so the timing is fixed; what plays isn't:
//! the audio draws its picks from a stream of its own, seeded here so a run repeats.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};
use squirrel::{Number, SqTable};

fn number(engine: &GameEngine, code: &str) -> f64 {
	engine.run_script(&format!("::__n <- {code};"), "audio").expect(code);
	SqTable::root(engine.vm()).get::<Number>("__n").expect("a number").0
}

#[test]
fn the_scripts_sound_plays_on_the_originals_rules() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	// The audio's stream mixes in the engine seed: 4 unless `BB_ENGINE_SEED` gives one
	// (4's ambience picks leave a gap at the test's last frame, which is why the checks
	// below watch every frame).
	let engine = harness::open_install_with(dir, |c| c.engine_seed = c.engine_seed.or(Some(4))).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");

	// The menu's track list: its first track, at once.
	let music: Vec<_> = engine.audio().playing().into_iter().filter(|p| p.channel == 0).collect();
	assert_eq!(music.len(), 1, "{music:?}");
	assert_eq!(music[0].file, "music/title.ogg");
	assert!(engine.audio().music_playing());

	// The defaults, and the music's volume stored halved but read back as set.
	assert_eq!(number(&engine, "::Settings.getSoundVolume(-1)"), 1.0);
	assert_eq!(number(&engine, "::Settings.getSoundVolume(0)"), f64::from(0.35f32) * 2.0);
	assert_eq!(number(&engine, "::Settings.getSoundVolume(1)"), f64::from(0.7f32));
	assert_eq!(number(&engine, "::Settings.getSoundVolume(2)"), f64::from(0.4f32));
	engine.run_script("::Settings.setSoundVolume(0, 0.5);", "volume").expect("set");
	assert_eq!(number(&engine, "::Settings.getSoundVolume(0)"), 0.5);

	// An effect plays its file through; a missing file plays nothing; with the effects
	// silenced nothing plays at all.
	engine
		.run_script(
			"::Sound.play(\"sounds/new_round_01.wav\", 1.0); ::Sound.play(\"sounds/no_such_sound.wav\", 1.0);",
			"effects",
		)
		.expect("play");
	let effects = || engine.audio().playing().into_iter().filter(|p| p.channel == 1).count();
	assert_eq!(effects(), 1);
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	assert_eq!(effects(), 0, "the 1.6 s sound ended and went");
	engine
		.run_script("::Settings.setSoundVolume(1, 0.0); ::Sound.play(\"sounds/new_round_01.wav\", 1.0, ::createVec(100, 200), 1.1);", "silent")
		.expect("play silent");
	assert_eq!(effects(), 0);
	engine.run_script("::Settings.setSoundVolume(1, 0.7);", "volume").expect("set");

	// The world: its ambience layers and its music. The first ambience waits for the
	// real clock to pass its 2.5 s minimum delay, and each sound is followed by a gap
	// (its length plus a second), so the ambience is watched over the frames, not
	// sampled at one; the menu's track fades out over 4 s.
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::unpause_world(&engine).expect("unpause");
	let mut ambience = Vec::new();
	for frame in 0..360 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
		for p in engine.audio().playing().into_iter().filter(|p| p.channel == 2) {
			if !ambience.contains(&p.file) {
				ambience.push(p.file);
			}
		}
	}
	assert!(!ambience.is_empty() && ambience.iter().all(|f| f.starts_with("sounds/ambience/")), "the ambience: {ambience:?}");
	let playing = engine.audio().playing();
	let music: Vec<_> = playing.iter().filter(|p| p.channel == 0).collect();
	assert_eq!(music.len(), 1, "one track at a time: {music:?}");
	assert_ne!(music[0].file, "music/title.ogg", "the menu's track has faded out for the world's");
	assert!(engine.audio().music_playing());
}
