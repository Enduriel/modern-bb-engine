//! The game's sounds through kira, on its mock backend (no device): a sound read from
//! the game's files plays once unpaused, for its length, and a track streams. Skips
//! cleanly when the Steam install isn't there.

use std::path::Path;
use std::sync::Arc;

use bb_audio::KiraOut;
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use kira::AudioManagerSettings;
use kira::backend::mock::{MockBackend, MockBackendSettings};
use squirrel_bb::AudioOut;

const RATE: u32 = 44_100;

/// Play `ms` of audio (the mock renders 128 frames a call).
fn run(out: &mut KiraOut<MockBackend>, ms: u32) {
	for _ in 0..(RATE * ms / 1000).div_ceil(128) {
		let backend = out.backend_mut();
		backend.on_start_processing();
		backend.process();
	}
}

#[test]
fn a_game_sound_plays_through_kira_for_its_length() {
	let dir = Path::new(STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let files = GameFiles::open(harness::data_archives(dir).expect("archives")).expect("open");
	let settings = AudioManagerSettings {
		backend_settings: MockBackendSettings { sample_rate: RATE },
		..AudioManagerSettings::default()
	};
	let mut out = KiraOut::new(settings, Arc::new(files), 0.3).expect("kira on the mock backend");

	assert_eq!(out.start("sounds/no_such_sound.wav", None), None);
	let length = out.length_ms("sounds/new_round_01.wav");
	assert!((1500..1700).contains(&length), "new_round_01 is about 1.6 s: {length}");
	let voice = out.start("sounds/new_round_01.wav", None).expect("starts");
	out.set_volume(voice, 0.7);
	run(&mut out, 200);
	assert_eq!(out.position_ms(voice), 0, "paused until told");
	out.set_paused(voice, false);
	run(&mut out, 500);
	let at = out.position_ms(voice);
	assert!((450..=550).contains(&at), "half a second in: {at}");
	assert!(!out.stopped(voice));
	run(&mut out, length);
	assert!(out.stopped(voice), "ended");

	// A track streams, read in place from its archive (vanilla's store their files
	// uncompressed) and decoded as it plays: none of its file is held.
	let track = out.length_ms("music/title.ogg");
	assert!(track > 60_000, "the title track is minutes long: {track}");
	let voice = out.start("music/title.ogg", None).expect("starts");
	out.set_paused(voice, false);
	// Its decoder reads ahead on a thread of its own: the position moves once it has.
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
	while out.position_ms(voice) == 0 {
		assert!(std::time::Instant::now() < deadline, "the track never played");
		run(&mut out, 10);
		std::thread::sleep(std::time::Duration::from_millis(5));
	}
	assert!(!out.stopped(voice));
	assert_eq!(out.bank_use().2, 0, "no file bytes held");
	out.stop(voice);
	assert!(out.stopped(voice));
}
