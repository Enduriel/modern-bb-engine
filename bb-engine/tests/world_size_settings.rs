//! The world map's size comes from vanilla's own settings, `Const.World.Settings.SizeX/
//! SizeY`, which the original honours only up to its 140 × 170 clamp (past it, it crashes;
//! a deliberate difference, see `docs/differences.md`):
//! - a mod that sets them as it loads (here a Modern Hooks mod made for the test) gets its
//!   size with no option;
//! - set at the main menu, they size the next campaign, without a restart, campaign after
//!   campaign;
//! - a mod that sets the world generator's `m.MinX/MinY` itself keeps its size.
//!
//! Skips without the Steam install (and the first, without Modern Hooks).

use std::io::Write;
use std::path::{Path, PathBuf};

use bb_engine::{harness, tactical_player as player};

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
const AT_MENU: &str = "::RootState.get(\"WorldState\") == null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible() && !::RootState.get(\"MainMenuState\").m.MainMenuScreen.isAnimating()";
const LOADED: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";

fn installed() -> bool {
	let there = Path::new(harness::STEAM_DATA_DIR).exists();
	if !there {
		eprintln!("skipping: {} not present", harness::STEAM_DATA_DIR);
	}
	there
}

fn frames(engine: &harness::GameEngine, n: u32) {
	for _ in 0..n {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap();
	}
}

fn until(engine: &harness::GameEngine, max: u32, what: &str, cond: &str) {
	for _ in 0..max {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap() == Some(1) {
			return;
		}
		frames(engine, 1);
	}
	panic!("{what}: still not true after {max} frames: {cond}");
}

/// The world map's size, as the scripts read it.
fn map_size(engine: &harness::GameEngine) -> String {
	player::eval_string(engine, "(function() { local s = ::World.getMapSize(); return s.X + \"x\" + s.Y; })()")
		.unwrap()
		.unwrap()
}

/// A new campaign from the main menu, as a player starts one.
fn start_from_menu(engine: &harness::GameEngine) {
	engine.run_script(&format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"), "new campaign").unwrap();
	frames(engine, 1);
	engine
		.run_script(
			&format!(
				"{MENU}.getNewCampaignMenuModule().onStartButtonPressed({{
					companyName = \"Sized\", banner = \"banner_04\",
					combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
					ironman = false, explorationDifficulty = false,
					lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
					seed = \"rust-engine-test\", origin = \"scenario.early_access\"
				}});"
			),
			"start",
		)
		.unwrap();
	until(engine, 600, "the campaign loading", LOADED);
}

/// A Modern Hooks mod that sets a 280 × 280 map from its queue, as a size mod would.
fn size_mod(dir: &Path) -> PathBuf {
	let path = dir.join("mod_zz_world_size_test.zip");
	let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
	zip.start_file("scripts/!mods_preload/mod_zz_world_size_test.nut", zip::write::SimpleFileOptions::default())
		.unwrap();
	zip.write_all(
		br#"local mod = ::Hooks.register("mod_zz_world_size_test", "1.0.0", "World size test");
mod.queue(function() {
	::Const.World.Settings.SizeX = 280;
	::Const.World.Settings.SizeY = 280;
});
"#,
	)
	.unwrap();
	zip.finish().unwrap();
	path
}

#[test]
fn a_mod_sets_the_size_as_it_loads() {
	if !installed() {
		return;
	}
	let data = Path::new(harness::STEAM_DATA_DIR);
	let Some(hooks) = harness::mod_archives(data)
		.unwrap()
		.into_iter()
		.find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("mod_modern_hooks")))
	else {
		eprintln!("skipping: Modern Hooks not installed");
		return;
	};
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("world_size_settings");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let mut archives = harness::data_archives(data).unwrap();
	archives.push(hooks);
	archives.push(size_mod(&dir));
	let engine = harness::open_mounted(data, archives, |_| {}).expect("open with the mod");
	harness::boot_menu(&engine).expect("boot the menu");
	// The mod's queue ran before MapGen built the generator, which copied its size as
	// vanilla's copies the settings (read past the engine's MapGen.get).
	let copied = player::eval_string(
		&engine,
		"(function() { local g = ::MapGen.m.Templates[\"world.worldmap_generator\"]; return g.m.MinX + \"x\" + g.m.MinY; })()",
	)
	.unwrap()
	.unwrap();
	assert_eq!(copied, "280x280", "copied as MapGen built the generator");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	assert_eq!(map_size(&engine), "280x280", "the mod's size, with no option");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_settings_size_each_new_campaign_without_a_restart() {
	if !installed() {
		return;
	}
	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	harness::install_autoplay(&engine).expect("autoplay");
	until(&engine, 600, "the main menu", AT_MENU);
	for (cols, rows) in [(200, 200), (280, 280), (140, 170)] {
		// As the console or a mod's settings would, at the main menu.
		engine
			.run_script(&format!("::Const.World.Settings.SizeX = {cols}; ::Const.World.Settings.SizeY = {rows};"), "size")
			.unwrap();
		start_from_menu(&engine);
		assert_eq!(map_size(&engine), format!("{cols}x{rows}"), "a campaign started after setting {cols} x {rows}");
		engine.run_script("::World.State.exitGame();", "quit to the menu").unwrap();
		until(&engine, 600, "back at the menu", AT_MENU);
	}
}

#[test]
fn a_mod_setting_the_generator_itself_keeps_its_size() {
	if !installed() {
		return;
	}
	let engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	// As World Parameters once did (in a startNewCampaign hook): the generator's own size,
	// the settings left alone.
	engine
		.run_script(
			"local g = ::MapGen.get(\"world.worldmap_generator\"); g.m.MinX = 160; g.m.MinY = 160;",
			"generator size",
		)
		.unwrap();
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	assert_eq!(map_size(&engine), "160x160");
}
