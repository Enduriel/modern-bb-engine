//! Boot a new campaign headless and render the world map around the player, like the
//! original's opening view (zoom 1), for comparing with its screenshots.
//!
//! ```text
//! cargo run --release -p bb-render --example render_world -- OUT.png [SEED] [WIDTHxHEIGHT]
//! ```

use std::path::Path;
use std::sync::Arc;

use bb_assets::{ImageLoader, Slot, TextureCache};
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::sprite::SpriteRenderer;
use bb_render::world::{WorldView, world_quads};
use bb_render::{Gpu, GpuTexture, RenderTarget, save_png};
use squirrel::{Number, SqStrongRef, SqTable};

fn main() {
	let mut args = std::env::args().skip(1);
	let out = args
		.next()
		.expect("usage: render_world OUT.png [SEED] [WxH]");
	let seed = args
		.next()
		.unwrap_or_else(|| "rust-engine-test".to_string());
	let (width, height) = args
		.next()
		.and_then(|s| {
			s.split_once('x')
				.map(|(w, h)| (w.parse().unwrap(), h.parse().unwrap()))
		})
		.unwrap_or((2560, 1440));

	let dir = Path::new(STEAM_DATA_DIR);
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("menu");
	harness::prepare_new_campaign(&engine, &seed).expect("prepare");
	harness::start_new_campaign(&engine).expect("start");
	// Let entities fade in (the original's 300 ms discovery fade).
	for _ in 0..40 {
		engine.frame(16.0).expect("frame");
	}
	engine
		.run_script(
			"::__playerPos <- ::World.State.getPlayer().getPos();",
			"player",
		)
		.expect("player");
	let pos = SqTable::root(engine.vm())
		.get::<SqStrongRef>("__playerPos")
		.expect("pos");
	let x = pos.get_member::<Number>(engine.vm(), "X").expect("x").0 as f32;
	let y = pos.get_member::<Number>(engine.vm(), "Y").expect("y").0 as f32;
	eprintln!("player at {x}, {y}");

	let scene = engine.world_scene();
	let view = WorldView {
		center: [x, y],
		zoom: 1.0,
		width,
		height,
	};
	let (quads, atlases) = engine
		.sprites()
		.with(|registry| world_quads(&scene, registry, &view));
	eprintln!("{} quads from {} atlases", quads.len(), atlases.paths.len());

	let gpu = Gpu::headless().expect("gpu");
	let files = Arc::new(GameFiles::open(harness::data_archives(dir).unwrap()).expect("files"));
	let mut cache: TextureCache<GpuTexture> = TextureCache::new(ImageLoader::new(files, 8));
	for path in &atlases.paths {
		cache.prefetch(path);
	}
	while atlases
		.paths
		.iter()
		.any(|p| matches!(cache.slot(p), Some(Slot::Loading)))
	{
		cache.pump(|path, image| GpuTexture::upload(&gpu, path, &image));
		std::thread::sleep(std::time::Duration::from_millis(2));
	}
	let textures: Vec<&GpuTexture> = atlases
		.paths
		.iter()
		.map(|p| match cache.slot(p) {
			Some(Slot::Ready(t)) => t,
			other => panic!("{p}: {other:?}", other = other.map(|_| "not ready")),
		})
		.collect();

	let renderer = SpriteRenderer::new(&gpu);
	let target = RenderTarget::new(&gpu, width, height);
	renderer.draw(
		&gpu,
		&target,
		&view.uniforms(),
		Some([0.0, 0.0, 0.0, 1.0]),
		&quads,
		&textures,
	);
	let image = target.read(&gpu).expect("read back");
	save_png(&out, &image).expect("save");
	eprintln!("wrote {out}");
}
