//! The sprite sheets of the installed game load the way the original loads them, and
//! scripts see them through `doesBrushExist`. Skips cleanly when the Steam install
//! isn't there.

use std::path::Path;

use bb_assets::name_hash;
use bb_engine::DatArchive;
use bb_engine::assets::load_sprites;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use squirrel::{SqArray, SqTable};

fn data_archives() -> Option<Vec<DatArchive>> {
	let dir = Path::new(STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {STEAM_DATA_DIR} not present");
		return None;
	}
	let paths = harness::data_archives(dir).expect("list archives");
	Some(paths.iter().map(|p| DatArchive::open(p).expect("open archive")).collect())
}

#[test]
fn every_installed_brush_loads() {
	let Some(mut archives) = data_archives() else { return };
	let load = load_sprites(&mut archives);
	assert!(load.failures.is_empty(), "{:?}", load.failures);

	let registry = &load.registry;
	let paths: Vec<&str> = registry.sheets().iter().map(|s| s.path.as_str()).collect();
	assert_eq!(paths.len(), 26);
	assert!(paths.is_sorted(), "one flat directory loads in sorted order: {paths:?}");
	// 7756 sprites, two of them repeated inside world_tiles.brush.
	assert_eq!(registry.len(), 7754);
	assert_eq!(registry.shadowed(), 2);

	for loaded in registry.sheets() {
		let texture = &loaded.sheet.texture;
		assert!(
			archives.iter_mut().any(|a| a.read(texture).is_some()),
			"{}: texture {texture} is in the archives",
			loaded.path
		);
		for sprite in &loaded.sheet.sprites {
			assert_eq!(sprite.hash, name_hash(sprite.name.as_bytes()), "{}", sprite.name);
			assert!(!sprite.frames.is_empty(), "{}", sprite.name);
			for frame in &sprite.frames {
				assert!(frame.uv.iter().all(|uv| (0.0..=1.0).contains(uv)), "{}", sprite.name);
			}
		}
	}
}

#[test]
fn a_known_sprite_reads_back_its_fields() {
	let Some(mut archives) = data_archives() else { return };
	let load = load_sprites(&mut archives);
	let (sheet, sprite) = load.registry.get("bust_head_dead_bloodpool").expect("registered");
	assert_eq!(sheet.path, "brushes/entity_0.brush");
	assert_eq!(sheet.sheet.texture, "gfx/entity_0.png");
	assert_eq!((sprite.width, sprite.height), (131, 125));
	assert_eq!((sprite.offset_x, sprite.offset_y), (6, 10));
	assert_eq!(sprite.unknown_offset, [-15.0, -15.0]);
}

#[test]
fn a_name_repeated_in_one_file_resolves_to_its_first_entry() {
	let Some(mut archives) = data_archives() else { return };
	let load = load_sprites(&mut archives);
	let id = load.registry.find("world_forest_needle_01").expect("registered");
	assert_eq!(load.registry.sheet(id).path, "brushes/world_tiles.brush");
	assert_eq!(id.sprite, 11, "the second copy is entry 26");
}

#[test]
fn scripts_see_the_loaded_brushes() {
	let dir = Path::new(STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {STEAM_DATA_DIR} not present");
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	engine
		.run_script(
			r#"::__brushes <- [
				doesBrushExist("bust_head_dead_bloodpool"),
				doesBrushExist("world_birds_01"),
				doesBrushExist(""),
				doesBrushExist("no_such_brush"),
				doesBrushExist("BUST_HEAD_DEAD_BLOODPOOL")
			];"#,
			"brushes",
		)
		.expect("run");
	let answers = SqTable::root(engine.vm()).get::<SqArray>("__brushes").expect("answers");
	let answers: Vec<bool> = (0..5).map(|i| answers.get::<bool>(i).expect("bool")).collect();
	assert_eq!(answers, [true, true, false, false, false]);
}
