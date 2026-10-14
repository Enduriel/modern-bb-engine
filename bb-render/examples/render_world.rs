//! Boot a new campaign headless and render the world map around the player, like the
//! original's opening view (zoom 1), for comparing with its screenshots.
//!
//! ```text
//! cargo run --release -p bb-render --example render_world -- OUT.png [SEED] [WIDTHxHEIGHT] [ZOOM] [X,Y]
//! ```
//!
//! `X,Y` centres the view there instead of on the player. `BB_NO_FOG=1` turns fog of
//! war off first, and the nearest shore tiles to the view are listed (for finding
//! water to compare).

use std::path::Path;
use std::sync::Arc;

use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::world::WorldView;
use bb_render::{Atlases, Clock, Gpu, RenderTarget, WorldRenderer, save_png};
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
	// The original's `World.getCamera().Zoom`: world units per screen pixel.
	let camera_zoom: f32 = args.next().map_or(1.0, |z| z.parse().expect("zoom"));
	let at: Option<[f32; 2]> = args.next().map(|s| {
		let (x, y) = s.split_once(',').expect("X,Y");
		[x.parse().expect("x"), y.parse().expect("y")]
	});

	let dir = Path::new(STEAM_DATA_DIR);
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("menu");
	harness::prepare_new_campaign(&engine, &seed).expect("prepare");
	harness::start_new_campaign(&engine).expect("start");
	engine
		.run_script(
			&format!("::World.getCamera().Zoom = {camera_zoom:?};"),
			"zoom",
		)
		.expect("zoom");
	// Let entities fade in (the original's 300 ms discovery fade), and scripts react to
	// the zoom.
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

	if std::env::var_os("BB_NO_FOG").is_some() {
		engine
			.run_script("::World.setFogOfWar(false);", "fog")
			.expect("fog off");
	}
	let scene = engine.world_scene();
	let center = at.unwrap_or([x, y]);
	let mut shores: Vec<_> = scene
		.tiles
		.iter()
		.filter(|t| t.terrain == 16)
		.map(|t| {
			let d = (t.pos.0 - f64::from(center[0])).hypot(t.pos.1 - f64::from(center[1]));
			(d as i64, t.pos)
		})
		.collect();
	shores.sort_by_key(|s| s.0);
	eprintln!("nearest shore tiles: {:?}", &shores[..shores.len().min(5)]);
	let view = WorldView {
		center,
		zoom: 1.0 / camera_zoom,
		width,
		height,
	};
	let gpu = Gpu::headless().expect("gpu");
	let files = Arc::new(GameFiles::open(harness::data_archives(dir).unwrap()).expect("files"));
	let mut renderer = WorldRenderer::new(&gpu, files, 8);
	if let Some(patch) = engine.world_grid().take_fog_patch() {
		renderer.update_fog(&gpu, &patch);
	}
	renderer.set_clock(Clock {
		real: engine.time().real_seconds() as f32,
		virtual_time: engine.time().virtual_seconds() as f32,
	});
	// The atlases the scripts' brushes asked for start loading now, as in a live game.
	let hints = engine.sprites().take_hints();
	eprintln!("{} atlases hinted", hints.len());
	renderer.prefetch(hints);
	let target = RenderTarget::new(&gpu, width, height);
	let stats = engine.sprites().with(|registry| {
		renderer.draw(&gpu, &target, &scene, registry, &view, Atlases::WaitForAll)
	});
	eprintln!("{stats:?}");
	let image = target.read(&gpu).expect("read back");
	save_png(&out, &image).expect("save");
	eprintln!("wrote {out}");
}
