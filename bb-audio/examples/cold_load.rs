//! How long a sound's first use costs on the game thread (reading it from the archives,
//! and decoding it if short), for the biggest ambience files and a few effects.
//!
//! ```text
//! cargo run --release -p bb-audio --example cold_load
//! ```

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use bb_audio::SoundBank;
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};

fn main() -> Result<(), String> {
	let dir = Path::new(STEAM_DATA_DIR);
	let io = |e: std::io::Error| e.to_string();
	let files = GameFiles::open(harness::data_archives(dir).map_err(io)?).map_err(io)?;
	let mut bank = SoundBank::new(Arc::new(files));
	for path in [
		"sounds/atmosphere/rain_01.wav",
		"sounds/ambience/terrain/forest_01.wav",
		"sounds/ambience/terrain/tundra_buzzard_06.wav",
		"sounds/ambience/terrain/mountain_wind_04.wav",
		"sounds/combat/riposte_01.wav",
		"sounds/new_round_01.wav",
		"music/worldmap_03.ogg",
	] {
		let t = Instant::now();
		let found = bank.get(path).is_some();
		let first = t.elapsed();
		let t = Instant::now();
		bank.get(path);
		println!("{path}: first use {:.1} ms, then {:.2} ms{}", first.as_secs_f64() * 1000.0, t.elapsed().as_secs_f64() * 1000.0, if found { "" } else { " (missing)" });
	}
	Ok(())
}
