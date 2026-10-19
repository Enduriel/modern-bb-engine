//! The world map can be any size (a deliberate difference, see `docs/differences.md`: the
//! original's `World.resizeScene` clamps it to 140 × 170 and its generator crashes past that). A new
//! campaign on a smaller (140 × 140) and a larger (280 × 280) map than vanilla's: worldgen
//! makes the map asked for, the fog of war mask keeps vanilla's texel size (2048² up to
//! the vanilla map, 4096² at twice its width), the world runs, and the campaign saves and
//! loads in our format. In the original's format (the default) both save and load too;
//! the larger map, which the original can't run, is marked so its load menu refuses it.
//! Vanilla's own 140 × 170 stays seed-exact (`world_boot`). Skips without the Steam
//! install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};
use squirrel_bb::SaveFormat;

/// The world as the scripts see it: the map's size, its settlements and entities.
const WORLD: &str = r#"(function() {
	local s = ::World.getMapSize();
	return s.X + "x" + s.Y + " settlements " + ::World.EntityManager.getSettlements().len()
		+ " entities " + ::World.getAllEntitiesAtPos(::createVec(0, 0), 10000000.0).len();
})()"#;

/// The magic our own save files start with (the original's are told apart by theirs).
const OWN_MAGIC: &[u8] = b"BBMESAVE";

fn frames(engine: &harness::GameEngine, n: u32) {
	for _ in 0..n {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap();
	}
}

fn campaign_on(cols: i64, rows: i64, mask: u32) {
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("world_sizes_{cols}x{rows}"));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	engine.saves().set_directory(dir.clone());
	harness::boot_menu(&engine).expect("boot the menu");
	harness::set_world_size(&engine, cols, rows).expect("world size");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare");
	harness::start_new_campaign(&engine).expect("startNewCampaign");

	let world = player::eval_string(&engine, WORLD).unwrap().unwrap();
	assert!(world.starts_with(&format!("{cols}x{rows} ")), "the map asked for: {world}");
	assert_eq!(engine.world_scene().tiles.len() as i64, cols * rows);
	let patch = engine.world_grid().take_fog_patch().expect("the new map's fog mask");
	assert_eq!((patch.mask, patch.width, patch.height), (mask, mask, mask), "{cols} x {rows}: its mask");

	harness::unpause_world(&engine).expect("unpause");
	frames(&engine, 600);
	engine.run_script("::World.State.setPause(true);", "pause").unwrap();
	let before = player::eval_string(&engine, WORLD).unwrap().unwrap();

	// Our format, and back.
	engine.saves().set_format(SaveFormat::Own);
	engine.run_script("::World.State.saveCampaign(\"sized\");", "save").unwrap();
	assert!(std::fs::read(dir.join("sized.sav")).unwrap().starts_with(OWN_MAGIC));
	frames(&engine, 300);
	engine.run_script("::World.State.loadCampaign(\"sized\");", "load").unwrap();
	assert_eq!(player::eval_string(&engine, WORLD).unwrap().unwrap(), before, "{cols} x {rows}: loaded");
	let patch = engine.world_grid().take_fog_patch().expect("the loaded fog mask");
	assert_eq!(patch.mask, mask, "the loaded mask keeps its size");

	// The original's format; past its map size, with the version's over-limit bit.
	engine.saves().set_format(SaveFormat::Original);
	engine.run_script("::World.State.saveCampaign(\"sized_original\");", "save").unwrap();
	let bytes = std::fs::read(dir.join("sized_original.sav")).unwrap();
	assert!(bytes.starts_with(&[0xbb, 0, 2]), "{cols} x {rows}: the original's container");
	let version = u32::from_le_bytes(bytes[3..7].try_into().unwrap());
	let over = cols > 140 || rows > 170;
	assert_eq!(version, if over { 0x0100_0000 | 65 } else { 65 }, "{cols} x {rows}: the version");
	frames(&engine, 300);
	engine.run_script("::World.State.loadCampaign(\"sized_original\");", "load").unwrap();
	let loaded = player::eval_string(&engine, WORLD).unwrap().unwrap();
	assert!(loaded.starts_with(&format!("{cols}x{rows} ")), "loaded again: {loaded}");
	// The scripts see the version they wrote, the bit cleared.
	let meta = "(function() { foreach (m in ::PersistenceManager.queryStorages()) if (m.getFileName() == \"sized_original\") return m.getVersion(); })()";
	assert_eq!(player::eval_i64(&engine, meta).unwrap(), Some(65));
	frames(&engine, 60);
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_smaller_map_than_vanilla() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		eprintln!("skipping: {} not present", harness::STEAM_DATA_DIR);
		return;
	}
	campaign_on(140, 140, 2048);
}

#[test]
fn a_larger_map_than_the_original_makes() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		eprintln!("skipping: {} not present", harness::STEAM_DATA_DIR);
		return;
	}
	campaign_on(280, 280, 4096);
}
