//! The game's sound on the default output device, without a window: boot the install
//! headless into a new campaign and run the world map in real time, printing what
//! plays each second.
//!
//! ```text
//! cargo run --release -p bb-audio --example listen -- [SECONDS] [--menu]
//! ```
//!
//! `--menu` stays on the main menu (its music). Each line also says how the device's
//! audio thread did (buffers filled, their load, underruns). To hear nothing, send it
//! to a null sink (`PIPEWIRE_NODE=<sink>`), or record it by pointing ALSA's default
//! device at a file (`ALSA_CONFIG_PATH`).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_audio::KiraOut;
use bb_audio::gains::rolloff_for_width;
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};

fn main() -> Result<(), String> {
	let mut args = std::env::args().skip(1);
	let seconds: u64 = args.next().map_or(Ok(10), |s| s.parse()).map_err(|_| "SECONDS")?;
	let menu = args.next().as_deref() == Some("--menu");
	let dir = Path::new(STEAM_DATA_DIR);
	let io = |e: std::io::Error| e.to_string();
	let engine = harness::open_install(dir).map_err(io)?;
	let files = GameFiles::open(harness::data_archives(dir).map_err(io)?).map_err(io)?;
	let (width, _, _) = engine.video().screen();
	let out = KiraOut::device(Arc::new(files), rolloff_for_width(width as u32))?;
	engine.audio().set_out(Box::new(out));
	harness::boot_menu(&engine)?;
	if !menu {
		harness::prepare_new_campaign(&engine, "rust-engine-test")?;
		harness::start_new_campaign(&engine)?;
		harness::unpause_world(&engine)?;
	}
	let frame = Duration::from_millis(16);
	let start = Instant::now();
	let mut last = Instant::now();
	let mut shown = 0;
	while start.elapsed().as_secs() < seconds {
		let now = Instant::now();
		harness::run_frame(&engine, (now - last).as_secs_f64() * 1000.0)?;
		last = now;
		if start.elapsed().as_secs() >= shown {
			shown += 1;
			let playing: Vec<String> = engine
				.audio()
				.playing()
				.iter()
				.map(|p| format!("{}[{}]{:.2}{}", p.file, p.channel, p.volume, if p.paused { " paused" } else { "" }))
				.collect();
			let status = engine.audio().output_status().unwrap_or_default();
			println!("{:>3}s [{status}] {}", start.elapsed().as_secs(), playing.join("  "));
		}
		std::thread::sleep(frame.saturating_sub(now.elapsed()));
	}
	Ok(())
}
