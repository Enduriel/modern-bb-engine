//! Boot a new campaign headless, start the battle the screenshot probes of the original
//! start (`battle_setup.nut`: bandits on `tactical.hills`, with the generators pinned
//! as the draw-exact battle scenarios (`bb-engine/tests/combat_scenarios.rs`) pin
//! them), and render the battle map with the camera on its centre tile and fog of war off.
//!
//! ```text
//! cargo run --release -p bb-render --example render_battle -- OUT.png [WIDTHxHEIGHT] [ZOOM]
//! ```
//!
//! `BB_LIST_UNITS=1` lists the units with their screen positions and bars.

use std::path::Path;
use std::sync::Arc;

use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::world::WorldView;
use bb_render::{Atlases, Gpu, RenderTarget, WorldRenderer, save_png};

/// The battle and the pilot, shared with the screenshot probe of the original.
const SETUP: &str = include_str!("battle_setup.nut");

/// One frame, with the UI client's replies and the pilot (event and combat dialog).
fn frame(engine: &harness::GameEngine) {
	engine.frame(1000.0 / 60.0).expect("frame");
	engine.pump_ui().expect("ui");
	engine.run_script("::__pilot();", "pilot").expect("pilot");
}

/// Whether one of the player's units is up.
fn player_turn(engine: &harness::GameEngine) -> bool {
	engine
		.run_script("::__turn <- ::__playerTurn();", "turn")
		.expect("turn");
	squirrel::SqTable::root(engine.vm()).get::<bool>("__turn") == Some(true)
}

fn main() {
	let mut args = std::env::args().skip(1);
	let out = args
		.next()
		.expect("usage: render_battle OUT.png [WxH] [ZOOM]");
	let (width, height) = args
		.next()
		.and_then(|s| {
			s.split_once('x')
				.map(|(w, h)| (w.parse().unwrap(), h.parse().unwrap()))
		})
		.unwrap_or((2560, 1440));
	let camera_zoom: f32 = args.next().map_or(1.0, |z| z.parse().expect("zoom"));

	let dir = Path::new(STEAM_DATA_DIR);
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare");
	harness::start_new_campaign(&engine).expect("start");
	engine.run_script(SETUP, "setup").expect("setup");
	// As the probe: two seconds on the world, then the battle.
	for _ in 0..120 {
		frame(&engine);
	}
	engine
		.run_script("::__startBattle();", "battle")
		.expect("battle");
	let mut ready = false;
	for _ in 0..6000 {
		frame(&engine);
		if player_turn(&engine) {
			ready = true;
			break;
		}
	}
	eprintln!("player's turn: {ready}");
	engine
		.run_script(
			&format!(
				"::__battleCamera(); ::Tactical.getCamera().Zoom = {camera_zoom:?}; ::__fogOff();"
			),
			"camera",
		)
		.expect("camera");
	for _ in 0..10 {
		frame(&engine);
	}

	let scene = engine.tactical_scene();
	eprintln!(
		"map {}x{}, camera {:?}, {} details",
		scene.width,
		scene.height,
		scene.camera,
		scene.tiles.iter().map(|t| t.details.len()).sum::<usize>()
	);
	if std::env::var_os("BB_LIST_UNITS").is_some() {
		for e in scene.entities.iter().filter(|e| e.is_actor) {
			let sx = (e.pos.0 as f32 - scene.camera.pos.0 as f32) * camera_zoom + width as f32 / 2.0;
			let sy = height as f32 / 2.0 - (e.pos.1 as f32 - scene.camera.pos.1 as f32) * camera_zoom;
			eprintln!("unit {} at {:?}, screen {sx},{sy}, overlay {:?}", e.id, e.pos, e.overlay);
		}
	}
	let view = WorldView {
		center: [scene.camera.pos.0 as f32, scene.camera.pos.1 as f32],
		zoom: 1.0 / camera_zoom,
		width,
		height,
	};
	let gpu = Gpu::headless().expect("gpu");
	let files = Arc::new(GameFiles::open(harness::data_archives(dir).unwrap()).expect("files"));
	let mut renderer = WorldRenderer::new(&gpu, files, 8);
	let target = RenderTarget::new(&gpu, width, height);
	let stats = engine.sprites().with(|registry| {
		renderer.draw_tactical(&gpu, &target, &scene, registry, &view, Atlases::WaitForAll)
	});
	eprintln!("{stats:?}");
	let image = target.read(&gpu).expect("read back");
	save_png(&out, &image).expect("save");
	eprintln!("wrote {out}");
}
