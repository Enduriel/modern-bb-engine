//! A frame copies only the tiles its view can show (`Engine::world_scene_where` with
//! `WorldView::shows`), and draws exactly what the whole map would: the same quads, from
//! the same atlases, for views across a new campaign's map. Skips cleanly when the Steam
//! install isn't there.

use std::path::Path;

use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::world::{WorldFonts, WorldView, world_quads};

#[test]
fn a_view_draws_the_same_from_only_the_tiles_it_shows() {
	let dir = Path::new(STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {STEAM_DATA_DIR} not present");
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare");
	harness::start_new_campaign(&engine).expect("start");
	let whole = engine.world_scene();
	let camera = engine.world_camera();
	let start = [camera.pos.0, camera.pos.1];
	let (w, h) = whole.extent;
	// The start, zoomed in and out, the map's corners and its middle.
	let views = [
		(start, 1.0 / camera.zoom),
		(start, 0.25),
		(start, 2.0),
		([0.0, 0.0], 0.5),
		([w, h], 0.5),
		([w / 2.0, h / 2.0], 0.4),
	];
	engine.sprites().with(|registry| {
		for (i, (center, zoom)) in views.into_iter().enumerate() {
			let view = WorldView {
				center,
				zoom,
				width: 2560,
				height: 1440,
			};
			let cut = engine.world_scene_where(|pos| view.shows(pos));
			assert!(cut.tiles.len() < whole.tiles.len() / 4, "a view shows a part of the map ({} tiles)", cut.tiles.len());
			let fonts = || WorldFonts { label: None, region: None };
			let all = world_quads(&whole, registry, &view, fonts());
			let some = world_quads(&cut, registry, &view, fonts());
			// The start is on land; a corner may be open sea, and the clouds over it drift
			// from an unseeded engine RNG.
			if i == 0 {
				assert!(!all.quads.is_empty(), "the start view draws something");
			}
			assert_eq!(some.atlases.paths, all.atlases.paths, "view at {center:?}, zoom {zoom}");
			assert_eq!(some.labels_from, all.labels_from);
			assert!(some.quads == all.quads, "view at {center:?}, zoom {zoom}: the same quads");
		}
	});
}
