//! Boot the real game to the main menu with the install's mods mounted: the
//! `data_*.dat` archives, then every mod `.zip` in the data dir, as the original
//! loads them. Prints the log lines that look like trouble and where the boot stopped.
//!
//! Run: `cargo run --release -p bb-engine --example boot_mods -- [data-dir]`

use std::path::PathBuf;

use bb_engine::harness;

fn main() {
	let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| harness::STEAM_DATA_DIR.to_string()));
	let mods = harness::mod_archives(&dir).expect("list mods");
	println!("{} mod archives:", mods.len());
	for m in &mods {
		println!("  {}", m.file_name().unwrap().to_string_lossy());
	}
	let mut engine = harness::open_install_with_mods(&dir).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	let start = std::time::Instant::now();
	match harness::boot_menu(&engine) {
		Ok(()) => println!("✅ menu booted in {:?}", start.elapsed()),
		Err(e) => println!("❌ boot failed after {:?}: {e}", start.elapsed()),
	}
	for frame in 0..120 {
		if let Err(e) = harness::run_frame(&engine, 1000.0 / 60.0) {
			println!("❌ frame {frame}: {e}");
			break;
		}
	}
	println!("states: {:?}", engine.root_state().state_names());
}
