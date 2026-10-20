//! Saves kept in a folder outlive the game: a campaign saved (and saved over), then the
//! engine gone, as when the game quits; a new engine on the same folder lists the save
//! in its Load menu and loads the campaign as it was. Deleting a save from the menus
//! takes its file away. The menus reach the saves through the original's natives
//! (`PersistenceManager`), so the vanilla UI works unchanged.
//!
//! Skips without the Steam install.

use std::path::{Path, PathBuf};

use bb_engine::{harness, tactical_player as player};

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";

/// What a loaded campaign must have as it was saved.
const SUMMARY: &str = r#"(function() {
	local t = ::World.getTime(), pos = ::World.State.getPlayer().getPos();
	local s = "day=" + t.Days + " time=" + t.Time.tointeger() + " money=" + ::World.Assets.getMoney() + " pos=" + pos.X.tointeger() + "," + pos.Y.tointeger();
	foreach (b in ::World.getPlayerRoster().getAll())
		s += " " + b.getName() + ":" + b.getHitpoints() + ":xp" + b.getXP() + ":items" + b.getItems().getAllItems().len();
	local items = [];
	foreach (it in ::World.Assets.getStash().getItems()) if (it != null) items.push(it.getID());
	items.sort();
	foreach (i in items) s += "," + i;
	return s;
})()"#;

/// A folder of its own for the test's saves, removed again by `Drop`.
struct Folder(PathBuf);

impl Drop for Folder {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.0);
	}
}

fn until(engine: &harness::GameEngine, cond: &str, frames: u32) {
	for _ in 0..frames {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap() == Some(1) {
			return;
		}
		harness::run_frame(engine, 1000.0 / 60.0).unwrap();
	}
	panic!("never: {cond}");
}

/// An engine over the install whose saves are kept in `dir`, at the main menu.
fn engine_at_menu(dir: &Path) -> harness::GameEngine {
	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	engine.saves().set_directory(dir.to_path_buf());
	harness::boot_menu(&engine).expect("boot the menu");
	engine
}

fn listed(engine: &harness::GameEngine) -> String {
	player::eval_string(engine, "(function() { local s = \"\"; foreach (c in ::UIDataHelper.convertCampaignStoragesToUIData()) s += c.fileName + \" \"; return s; })()")
		.unwrap()
		.unwrap()
}

#[test]
fn a_campaign_saved_to_disk_loads_in_a_new_game() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		eprintln!("skipping: no Steam install");
		return;
	}
	let folder = Folder(std::env::temp_dir().join(format!("bb-saves-on-disk-{}", std::process::id())));
	let _ = std::fs::remove_dir_all(&folder.0);

	let saved = {
		let engine = engine_at_menu(&folder.0);
		harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
		harness::start_new_campaign(&engine).expect("startNewCampaign");
		harness::unpause_world(&engine).expect("unpause");
		for _ in 0..300 {
			harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
		}
		engine.run_script("::World.State.setPause(true); ::World.State.saveCampaign(\"bb_probe_disk\");", "save").expect("save");
		assert!(folder.0.join("bb_probe_disk.sav").is_file(), "the save is a file");
		// Saved over: the newer campaign is the one kept.
		engine.run_script("::World.Assets.addMoney(1234); ::World.State.saveCampaign(\"bb_probe_disk\"); ::World.State.saveCampaign(\"bb_probe_spare\");", "save again").expect("save again");
		assert_eq!(listed(&engine), "bb_probe_disk bb_probe_spare ", "both saves listed");
		// Deleted from the menus: the file goes.
		engine.run_script("::PersistenceManager.deleteStorage(\"bb_probe_spare\");", "delete").expect("delete");
		assert!(!folder.0.join("bb_probe_spare.sav").exists(), "the deleted save's file is gone");
		player::eval_string(&engine, SUMMARY).unwrap().unwrap()
		// The engine goes here, as when the game quits.
	};

	let engine = engine_at_menu(&folder.0);
	assert_eq!(listed(&engine), "bb_probe_disk ", "the new game lists the save");
	// The world state ignores a load within 4 s (real time) of the last, the game's
	// start counting as one: a player can't be at the menu that soon in the original.
	while player::eval_i64(&engine, "::Time.getRealTimeF() > 4.5 ? 1 : 0").unwrap() != Some(1) {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	engine.run_script(&format!("{MENU}.getMainMenuModule().onLoadCampaignButtonPressed();"), "load menu").expect("load menu");
	harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	engine.run_script(&format!("{MENU}.getLoadCampaignMenuModule().onLoadButtonPressed(\"bb_probe_disk\");"), "load").expect("load");
	until(
		&engine,
		"::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()",
		1200,
	);
	let loaded = player::eval_string(&engine, SUMMARY).unwrap().unwrap();
	assert_eq!(loaded, saved, "the loaded campaign is the one saved");
}
