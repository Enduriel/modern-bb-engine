//! The world map's frame cost while the camera pans, headless: boots a new campaign,
//! then draws frames as `bb` does (the scene cut to the view, atlases as ready) while
//! the camera sweeps the map at zoom 1 and fully zoomed out, and prints each phase's
//! frame times by span (see `bb_render::frame_timing`), the GPU's included (each frame
//! waits for it).
//!
//! ```text
//! cargo run --release -p bb-render --example world_pan -- [WIDTHxHEIGHT] [FRAMES] [--mods] [--shot OUT.png]
//! ```
//!
//! `--shot` saves one frame, fully zoomed out on a fixed spot with every atlas waited
//! for, to compare the drawing before and after a change (`BB_ENGINE_SEED` fixes the
//! engine's randomness, e.g. the details' mirroring).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::world::WorldView;
use bb_render::{Atlases, Clock, Gpu, RenderTarget, WorldRenderer, frame_timing, save_png};

fn main() {
	let mut args: Vec<String> = std::env::args().skip(1).collect();
	let mods = take_flag(&mut args, "--mods");
	let shot = take_value(&mut args, "--shot");
	let (width, height) = args
		.first()
		.and_then(|s| s.split_once('x').map(|(w, h)| (w.parse().unwrap(), h.parse().unwrap())))
		.unwrap_or((2560u32, 1358u32));
	let frames: usize = args.get(1).map_or(600, |n| n.parse().expect("FRAMES"));

	let dir = Path::new(STEAM_DATA_DIR);
	let engine = if mods { harness::open_install_with_mods(dir) } else { harness::open_install(dir) }.expect("open install");
	engine.video().set_screen_size(i64::from(width), i64::from(height));
	harness::boot_menu(&engine).expect("menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare");
	harness::start_new_campaign(&engine).expect("start");
	for _ in 0..40 {
		engine.frame(16.0).expect("frame");
	}
	let mut archives = harness::data_archives(dir).unwrap();
	if mods {
		archives.extend(harness::mod_archives(dir).unwrap());
	}
	let gpu = Gpu::headless().expect("gpu");
	let mut renderer = WorldRenderer::new(&gpu, Arc::new(GameFiles::open(archives).expect("files")), 8);
	let target = RenderTarget::new(&gpu, width, height);
	let center = engine.world_camera().pos;
	frame_timing::enable();

	for (name, zoom, reach) in [("zoom 1", 1.0f32, 1500.0f32), ("zoomed out", 100.0, 6000.0)] {
		engine.run_script(&format!("::World.getCamera().Zoom = {zoom:?};"), "zoom").expect("zoom");
		let mut spans: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
		let mut quads = Vec::new();
		for i in 0..frames {
			let t = i as f32;
			let x = center.0 + reach * (std::f32::consts::TAU * t / 240.0).sin();
			let y = center.1 + reach * 0.5 * (std::f32::consts::TAU * t / 360.0).cos();
			engine
				.run_script(&format!("::World.getCamera().setPos(::createVec({x:?}, {y:?}));"), "pan")
				.expect("pan");
			let started = Instant::now();
			{
				let _span = frame_timing::span("step");
				engine.frame(16.0).expect("frame");
			}
			let stats = draw(&engine, &gpu, &mut renderer, &target, Atlases::DrawReady);
			{
				let _span = frame_timing::span("gpu");
				gpu.wait().expect("gpu");
			}
			let total = started.elapsed();
			let taken = frame_timing::take();
			// The first frames load the view's frames: not what panning costs.
			if i < 60 {
				continue;
			}
			spans.entry("frame").or_default().push(ms(total));
			for (span, time) in taken {
				spans.entry(span).or_default().push(ms(time));
			}
			quads.push(stats.quads as f64);
		}
		let zoom_now = engine.world_camera().zoom;
		println!("== {name} (camera zoom {zoom_now:.2}), {width}x{height}, {} frames", frames.saturating_sub(60));
		print_row("quads", &mut quads);
		for (span, times) in &mut spans {
			print_row(span, times);
		}
	}

	if let Some(path) = shot {
		engine.run_script("::World.getCamera().Zoom = 100.0;", "zoom").expect("zoom");
		engine
			.run_script(&format!("::World.getCamera().setPos(::createVec({:?}, {:?}));", center.0, center.1), "pan")
			.expect("pan");
		engine.frame(16.0).expect("frame");
		draw(&engine, &gpu, &mut renderer, &target, Atlases::WaitForAll);
		save_png(&path, &target.read(&gpu).expect("read back")).expect("save");
		eprintln!("wrote {path}");
	}
}

/// One frame of the world as `bb` draws it.
fn draw(
	engine: &harness::GameEngine,
	gpu: &Gpu,
	renderer: &mut WorldRenderer,
	target: &RenderTarget,
	atlases: Atlases,
) -> bb_render::FrameStats {
	renderer.begin_frame();
	let hints = engine.sprites().take_hints();
	engine.sprites().with(|registry| renderer.prefetch(registry, hints));
	if let Some(patch) = engine.world_grid().take_fog_patch() {
		renderer.update_fog(gpu, &patch);
	}
	renderer.set_clock(Clock {
		real: engine.time().real_seconds() as f32,
		virtual_time: engine.time().virtual_seconds() as f32,
	});
	let camera = engine.world_camera();
	let view = WorldView {
		center: [camera.pos.0, camera.pos.1],
		zoom: 1.0 / camera.zoom,
		width: target.width,
		height: target.height,
	};
	let scene = {
		let _span = frame_timing::span("world scene");
		engine.world_scene_where(|pos| view.shows(pos))
	};
	let _span = frame_timing::span("world draw");
	engine.sprites().with(|registry| renderer.draw(gpu, target, &scene, registry, &view, atlases))
}

fn ms(d: Duration) -> f64 {
	d.as_secs_f64() * 1000.0
}

fn print_row(name: &str, values: &mut [f64]) {
	if values.is_empty() {
		return;
	}
	values.sort_by(f64::total_cmp);
	let at = |p: f64| values[((values.len() as f64 * p) as usize).min(values.len() - 1)];
	let mean = values.iter().sum::<f64>() / values.len() as f64;
	println!("  {name:14} mean {mean:8.3}  p50 {:8.3}  p90 {:8.3}  p99 {:8.3}  max {:8.3}", at(0.5), at(0.9), at(0.99), values[values.len() - 1]);
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
	let found = args.iter().position(|a| a == flag);
	found.map(|i| args.remove(i)).is_some()
}

fn take_value(args: &mut Vec<String>, flag: &str) -> Option<String> {
	let i = args.iter().position(|a| a == flag)?;
	args.remove(i);
	(i < args.len()).then(|| args.remove(i))
}
