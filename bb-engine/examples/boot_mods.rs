//! Boot the real game to the main menu with the install's mods mounted: the
//! `data_*.dat` archives, then every mod `.zip` in the data dir, as the original
//! loads them. Prints the log lines that look like trouble and where the boot stopped.
//!
//! Run: `cargo run --release -p bb-engine --example boot_mods -- [data-dir]`
//!
//! - `BOOT_MODS_ONLY=mod_modern_hooks,mod_msu` mounts only the mods whose file names
//!   start with one of those prefixes.
//! - `BOOT_MODS_EXTRA=/path/a.zip,/path/b.zip` also mounts these zips from anywhere (e.g.
//!   a dev build): in load order by file name among the data dir's mods, as the original
//!   orders them once copied into `data/` (what the probe runner's `--with-mod` does). One
//!   named like an installed mod replaces it. `BOOT_MODS_ONLY` doesn't filter them.
//! - `BOOT_MODS_SCRIPT=probe.nut` runs a script once the menu is up (e.g. a probe
//!   written for the original, to compare).
//! - `BOOT_MODS_CAMPAIGN=1` then starts a new campaign through the menus (as the smoke
//!   test does) and lets the world run unpaused for 10 s.

use std::path::PathBuf;

use bb_engine::{harness, tactical_player};

fn main() {
	let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| harness::STEAM_DATA_DIR.to_string()));
	let mut mods = harness::mod_archives(&dir).expect("list mods");
	if let Ok(only) = std::env::var("BOOT_MODS_ONLY") {
		let prefixes: Vec<&str> = only.split(',').collect();
		mods.retain(|m| {
			let name = m.file_name().unwrap().to_string_lossy();
			prefixes.iter().any(|p| name.starts_with(p))
		});
	}
	let extra: Vec<PathBuf> = std::env::var("BOOT_MODS_EXTRA")
		.map(|list| list.split(',').map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).collect())
		.unwrap_or_default();
	for zip in &extra {
		assert!(zip.is_file(), "BOOT_MODS_EXTRA: not a file: {}", zip.display());
	}
	let mods = harness::with_extra_mods(mods, extra.clone()).expect("BOOT_MODS_EXTRA");
	println!("{} mod archives:", mods.len());
	for m in &mods {
		let from = if extra.contains(m) { format!("  (extra: {})", m.display()) } else { String::new() };
		println!("  {}{from}", m.file_name().unwrap().to_string_lossy());
	}
	let mut archives = harness::data_archives(&dir).expect("list the data archives");
	archives.extend(mods);
	let mut engine = harness::open_mounted(&dir, archives, |_| {}).expect("open install");
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
	if let Ok(path) = std::env::var("BOOT_MODS_SCRIPT") {
		let script = std::fs::read_to_string(&path).expect("read the script");
		if let Err(e) = engine.run_script(&script, &path) {
			println!("❌ {path}: {e}");
		}
	}
	if std::env::var("BOOT_MODS_CAMPAIGN").is_ok() {
		start_campaign(&engine);
	}
}

/// Main menu → New Campaign → Start, then the world unpaused for 600 frames.
fn start_campaign(engine: &harness::GameEngine) {
	const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
	let step = |what: &str, src: &str| -> bool {
		match engine.run_script(src, what) {
			Ok(()) => true,
			Err(e) => {
				println!("❌ {what}: {e}");
				false
			}
		}
	};
	let frames = |n: u32| -> bool {
		for frame in 0..n {
			if let Err(e) = harness::run_frame(engine, 1000.0 / 60.0) {
				println!("❌ frame {frame}: {e}");
				return false;
			}
		}
		true
	};
	let start = std::time::Instant::now();
	if !step("new campaign", &format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();")) || !frames(1) {
		return;
	}
	let settings = r#"{
		companyName = "Corpus Company", banner = "banner_04",
		combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
		ironman = false, explorationDifficulty = false,
		lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
		seed = "rust-engine-test", origin = "scenario.early_access"
	}"#;
	if !step("start", &format!("{MENU}.getNewCampaignMenuModule().onStartButtonPressed({settings});")) {
		return;
	}
	let started = "(\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";
	// MSU asks for its new-campaign settings first: keep the defaults (Save).
	let msu_settings = "(\"MSU\" in ::getroottable()) && ::MSU.SettingsScreen.isVisible()";
	for _ in 0..600 {
		if tactical_player::eval_i64(engine, &format!("({started}) ? 1 : 0")).ok().flatten() == Some(1) {
			break;
		}
		if tactical_player::eval_i64(engine, &format!("({msu_settings}) ? 1 : 0")).ok().flatten() == Some(1)
			&& !step("MSU settings: save", "::MSU.SettingsScreen.onSaveButtonPressed({});")
		{
			return;
		}
		if !frames(1) {
			return;
		}
	}
	if tactical_player::eval_i64(engine, &format!("({started}) ? 1 : 0")).ok().flatten() != Some(1) {
		println!("❌ the campaign didn't start in 600 frames");
		return;
	}
	println!("✅ campaign started in {:?}", start.elapsed());
	if step("unpause", "::World.State.setAutoPause(false); ::World.State.setPause(false);") && frames(600) {
		println!("✅ the world ran 600 frames");
	}
}
