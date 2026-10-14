//! The mod frameworks run unchanged. Each test mounts one framework from the
//! install's mods over the vanilla archives, as the original loads them, and boots the
//! main menu. Skips when the Steam install or the mod is missing (the mods are the
//! user's, read-only; a running game probe holds them aside in
//! `_bb_probe_held_mods/`, so that folder is searched too).

use std::path::{Path, PathBuf};

use bb_engine::{harness, tactical_player as player};

/// The installed mod archive whose name starts with `prefix`, if any.
fn find_mod(prefix: &str) -> Option<PathBuf> {
	let data = Path::new(harness::STEAM_DATA_DIR);
	let held = data.parent()?.join("_bb_probe_held_mods");
	[data.to_path_buf(), held].iter().find_map(|dir| {
		let mut zips: Vec<PathBuf> = std::fs::read_dir(dir)
			.ok()?
			.flatten()
			.map(|e| e.path())
			.filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix)))
			.collect();
		zips.sort();
		zips.pop()
	})
}

/// An engine over the vanilla archives plus the mods named by `prefixes`, or `None` (and
/// a skip notice) when the install or one of the mods is missing.
fn open_with(prefixes: &[&str]) -> Option<harness::GameEngine> {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return None;
	}
	let mut archives = harness::data_archives(dir).expect("list the data archives");
	for prefix in prefixes {
		let Some(zip) = find_mod(prefix) else {
			eprintln!("skipping: no {prefix}* mod installed");
			return None;
		};
		archives.push(zip);
	}
	Some(harness::open_mounted(dir, archives, |_| {}).expect("open the archives"))
}

fn check(engine: &harness::GameEngine, what: &str, expr: &str) {
	let v = player::eval_i64(engine, &format!("({expr}) ? 1 : 0"))
		.unwrap_or_else(|e| panic!("{what}: {e}"));
	assert_eq!(v, Some(1), "{what}: {expr}");
}

/// Modern Hooks holds back root_state's and the main menu's onInit until its UI
/// handshake (answered by the headless UI client), then runs them: the menu comes up
/// with vanilla's UI globals, and the stdlib `split` survives `skills/actives/split`.
#[test]
fn modern_hooks_boots_the_menu() {
	let Some(engine) = open_with(&["mod_modern_hooks"]) else { return };
	harness::boot_menu(&engine).expect("boot the menu");
	for _ in 0..10 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	check(&engine, "MH registered itself", "::Hooks.hasMod(\"mod_modern_hooks\")");
	check(&engine, "root_state's own onInit ran", "\"Cursor\" in ::getroottable() && ::Root == ::Hooks.RootState");
	check(&engine, "the main menu's own onInit ran", "::Hooks.MainMenuState.m.MainMenuScreen != null");
	check(&engine, "the stdlib split survives", "typeof ::split == \"function\" && ::split(\"a,b\", \",\").len() == 2");
	check(&engine, "MH found vanilla's split class", "\"scripts/skills/actives/split\" in ::Hooks.BBClass");
	check(&engine, "its UI files are listed", "::IO.enumerateFiles(\"ui/mods/\").find(\"ui/mods/modern_hooks/main\") != null");
}

/// The whole installed corpus (every mod in the data dir, as the original loads them):
/// Modern Hooks registers every class it saw (3,614 for the pinned 32 mods, as in the
/// original), and a new campaign starts through the menus and runs. Skips without mods.
#[test]
fn the_installed_mods_boot_and_start_a_campaign() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	let Ok(mods) = harness::mod_archives(dir) else {
		eprintln!("skipping: {} not present", dir.display());
		return;
	};
	if !mods.iter().any(|m| m.file_name().is_some_and(|n| n.to_string_lossy().starts_with("mod_modern_hooks"))) {
		eprintln!("skipping: Modern Hooks isn't installed");
		return;
	}
	let mut archives = harness::data_archives(dir).expect("list the data archives");
	archives.extend(mods);
	let engine = harness::open_mounted(dir, archives, |_| {}).expect("open the archives");
	harness::boot_menu(&engine).expect("boot the menu");
	for _ in 0..10 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	check(&engine, "every class registered with Modern Hooks", "(function() { foreach (c in ::Hooks.BBClass) if (c.Prototype == null) return false; return true; })()");

	const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
	engine.run_script(&format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"), "new campaign").expect("new campaign");
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	engine
		.run_script(
			&format!(
				"{MENU}.getNewCampaignMenuModule().onStartButtonPressed({{
					companyName = \"Corpus Company\", banner = \"banner_04\",
					combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
					ironman = false, explorationDifficulty = false,
					lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
					seed = \"rust-engine-test\", origin = \"scenario.early_access\"
				}});"
			),
			"start",
		)
		.expect("start");
	let started = "(\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";
	let msu_settings = "(\"MSU\" in ::getroottable()) && ::MSU.SettingsScreen.isVisible()";
	let flag = |expr: &str| player::eval_i64(&engine, &format!("({expr}) ? 1 : 0")).expect(expr) == Some(1);
	for _ in 0..600 {
		if flag(started) {
			break;
		}
		// MSU asks for its new-campaign settings first: keep the defaults.
		if flag(msu_settings) {
			engine.run_script("::MSU.SettingsScreen.onSaveButtonPressed({});", "MSU settings").expect("MSU settings");
		}
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	assert!(flag(started), "the campaign didn't start");
	engine.run_script("::World.State.setAutoPause(false); ::World.State.setPause(false);", "unpause").expect("unpause");
	for _ in 0..300 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
}
