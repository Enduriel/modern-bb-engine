//! Load a campaign save the original wrote from the main menu's Load Campaign,
//! then let the world run, and say what came of it.
//!
//! Run: `cargo run --release -p bb-engine --example load_original_save -- <file.sav> [--mods]`
//!
//! The save is copied to `target/tmp/load_original_save` first (the game writes beside
//! it); the original file is only read. `--mods` mounts every mod in the install, as a save
//! made with mods needs.

use std::path::{Path, PathBuf};

use bb_engine::{harness, tactical_player as player};

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
const LOADED: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";
const SUMMARY: &str = r#"(function() {
	local t = ::World.getTime();
	return "day " + t.Days + ", " + ::World.Assets.getName() + ": " + ::World.getPlayerRoster().getSize() + " brothers, "
		+ ::World.Assets.getMoney() + " crowns, " + ::World.getAllEntitiesAtPos(::createVec(0, 0), 10000000.0).len() + " world entities";
})()"#;

fn frame(engine: &harness::GameEngine) -> Result<(), String> {
	harness::run_frame(engine, 1000.0 / 60.0)
}

fn until(engine: &harness::GameEngine, max: u32, cond: &str) -> Result<bool, String> {
	for _ in 0..max {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0"))? == Some(1) {
			return Ok(true);
		}
		frame(engine)?;
	}
	Ok(false)
}

fn run(save: &Path, mods: bool) -> Result<(), String> {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	let io = |e: std::io::Error| e.to_string();
	let mut archives = harness::data_archives(dir).map_err(io)?;
	if mods {
		archives.extend(harness::mod_archives(dir).map_err(io)?);
	}
	let mut engine = harness::open_mounted(dir, archives, |_| {}).map_err(io)?;
	engine.vm_mut().install_standard_error_handlers();
	let saves = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/tmp/load_original_save");
	std::fs::create_dir_all(&saves).map_err(|e| e.to_string())?;
	let name = save.file_stem().ok_or("no file name")?.to_string_lossy().into_owned();
	std::fs::copy(save, saves.join(format!("{name}.sav"))).map_err(|e| format!("copying the save: {e}"))?;
	engine.saves().set_directory(saves.clone());
	engine.saves().set_modified(mods);

	harness::boot_menu(&engine)?;
	until(&engine, 600, &format!("{MENU}.isVisible() && !{MENU}.isAnimating()"))?;
	// The world state ignores a load within 4 s of the game's start.
	until(&engine, 600, "::Time.getRealTimeF() > 4.5")?;
	engine.run_script(&format!("{MENU}.getMainMenuModule().onLoadCampaignButtonPressed();"), "load menu")?;
	frame(&engine)?;
	let start = std::time::Instant::now();
	engine.run_script(&format!("{MENU}.getLoadCampaignMenuModule().onLoadButtonPressed(\"{name}\");"), "load")?;
	if !until(&engine, 600, LOADED)? {
		return Err("the campaign didn't load".into());
	}
	println!("✅ loaded in {:?}: {}", start.elapsed(), player::eval_string(&engine, SUMMARY)?.unwrap_or_default());
	engine.run_script("::World.State.setAutoPause(false); ::World.State.setPause(false);", "unpause")?;
	for _ in 0..1200 {
		frame(&engine)?;
	}
	println!("✅ ran 20 s: {}", player::eval_string(&engine, SUMMARY)?.unwrap_or_default());
	let _ = std::fs::remove_dir_all(&saves);
	Ok(())
}

fn main() {
	let mut args = std::env::args().skip(1);
	let save = PathBuf::from(args.next().expect("usage: load_original_save <file.sav> [--mods]"));
	let mods = args.any(|a| a == "--mods");
	if let Err(e) = run(&save, mods) {
		println!("❌ {e}");
		std::process::exit(1);
	}
}
