//! Boot a new campaign headless and compose the portraits the UI asks for
//! (`coui://procedural/tacticalentity(...)`) of the player's brothers, as PNGs; with
//! `BB_WORLD=1`, world entities' (`worldentity(...)`).
//!
//! ```text
//! cargo run --release -p bb-render --example render_portrait -- OUT_DIR [COUNT]
//! ```

use std::path::Path;
use std::sync::Arc;

use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::portrait::PortraitRequest;
use bb_render::{Gpu, WorldRenderer, save_png};
use squirrel::{SqStrongRef, SqTable};

fn main() {
	let mut args = std::env::args().skip(1);
	let out = args.next().expect("usage: render_portrait OUT_DIR [COUNT]");
	let count: usize = args.next().map_or(3, |c| c.parse().expect("count"));
	let dir = Path::new(STEAM_DATA_DIR);
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare");
	harness::start_new_campaign(&engine).expect("start");
	// The paths the brothers' getImagePath() gives the UI; with BB_WORLD=1, the world
	// entities' instead (`worldentity(...)`): the player's party, then settlements and
	// locations, alternating.
	let script = if std::env::var_os("BB_WORLD").is_some() {
		r#"::__paths <- [::World.State.getPlayer().getImagePath()];
		local s = ::World.EntityManager.getSettlements(), l = ::World.EntityManager.getLocations();
		for (local i = 0; i < 20; i++) {
			if (i < s.len()) ::__paths.push("worldentity(0," + s[i].getID() + ")");
			if (i < l.len()) ::__paths.push(l[i].getImagePath());
		}"#
	} else {
		"::__paths <- ::World.getPlayerRoster().getAll().map(@(b) b.getImagePath());"
	};
	engine.run_script(script, "paths").expect("paths");
	let paths = SqTable::root(engine.vm()).get::<SqStrongRef>("__paths").expect("paths");
	let paths: Vec<String> = (0..count)
		.filter_map(|i| paths.get_value::<squirrel::SqArray>(engine.vm()).and_then(|a| a.get::<String>(i)))
		.collect();

	let gpu = Gpu::headless().expect("gpu");
	let files = Arc::new(GameFiles::open(harness::data_archives(dir).unwrap()).expect("files"));
	let mut renderer = WorldRenderer::new(&gpu, files, 8);
	std::fs::create_dir_all(&out).expect("out dir");
	for (i, path) in paths.iter().enumerate() {
		let request = PortraitRequest::parse(path).expect("a procedural path");
		let mut portrait = engine.portrait(request.kind, request.id).expect("the entity");
		// BB_SHOW=a,b: force those sprites visible (to compare with the original).
		if let Ok(show) = std::env::var("BB_SHOW") {
			for s in portrait.sprites.iter_mut().filter(|s| show.split(',').any(|n| n == s.name)) {
				s.visible = true;
			}
		}
		// BB_LIST_SPRITES=1: what the portrait draws from.
		if std::env::var_os("BB_LIST_SPRITES").is_some() {
			for s in &portrait.sprites {
				eprintln!("  {} {:?} visible {} in_portrait {} offset {:?}", s.name, s.brush, s.visible, s.in_portrait, s.offset);
			}
		}
		let image = engine
			.sprites()
			.with(|registry| renderer.compose_portrait(&gpu, &portrait, &request.hidden, registry))
			.expect("compose");
		match image {
			Some(image) => {
				let file = format!("{out}/portrait_{i}.png");
				save_png(&file, &image).expect("save");
				eprintln!("{path}: {}x{} -> {file}", image.width, image.height);
			}
			None => eprintln!("{path}: nothing to draw"),
		}
	}
}
