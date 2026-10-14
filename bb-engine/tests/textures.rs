//! The installed game's atlases load on demand through the texture cache, and a missing
//! one takes the game's "texture missing" picture. Skips cleanly when the Steam install
//! isn't there.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_assets::{ImageLoader, Slot, TextureCache};
use bb_engine::DatArchive;
use bb_engine::assets::load_sprites;
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};

fn install() -> Option<Vec<std::path::PathBuf>> {
	let dir = Path::new(STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {STEAM_DATA_DIR} not present");
		return None;
	}
	Some(harness::data_archives(dir).expect("list archives"))
}

#[test]
fn every_atlas_decodes_at_its_sheet_size() {
	let Some(paths) = install() else { return };
	let mut archives: Vec<DatArchive> = paths.iter().map(|p| DatArchive::open(p).unwrap()).collect();
	let registry = load_sprites(&mut archives).registry;
	let files = Arc::new(GameFiles::open(&paths).expect("open archives"));
	let mut cache: TextureCache<(u32, u32)> = TextureCache::new(ImageLoader::new(files, 4));

	for loaded in registry.sheets() {
		cache.prefetch(&loaded.sheet.texture);
	}
	let deadline = Instant::now() + Duration::from_secs(120);
	while registry.sheets().iter().any(|s| matches!(cache.slot(&s.sheet.texture), Some(Slot::Loading))) {
		assert!(Instant::now() < deadline, "atlases still loading");
		cache.pump(|_, image| {
			assert_eq!(image.rgba.len(), image.width as usize * image.height as usize * 4);
			(image.width, image.height)
		});
		std::thread::sleep(Duration::from_millis(5));
	}
	for loaded in registry.sheets() {
		let header = &loaded.sheet.header;
		assert_eq!(
			cache.slot(&loaded.sheet.texture),
			Some(&Slot::Ready((u32::from(header.width), u32::from(header.height)))),
			"{}",
			loaded.path
		);
	}
}

/// A texture that isn't in the game's files takes the game's own "texture missing"
/// picture, `error.png` (128×128, at the archives' root), as the original's do.
#[test]
fn a_missing_texture_takes_the_games_texture_missing_picture() {
	let Some(paths) = install() else { return };
	let files = Arc::new(GameFiles::open(&paths).expect("open archives"));
	let mut cache: TextureCache<(u32, u32)> = TextureCache::new(ImageLoader::new(files, 1)).with_fallback("error.png");
	let missing = cache.get_now("gfx/no_such_atlas.png", |_, image| (image.width, image.height));
	assert_eq!(missing, Ok(&(128, 128)));
	assert!(matches!(cache.slot("gfx/no_such_atlas.png"), Some(Slot::Missing(..))));
}
