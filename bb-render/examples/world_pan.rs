//! The world map's frame cost while the camera pans, headless: boots a new campaign,
//! then draws frames as `bb` does (the scene cut to the view, atlases as ready) while
//! the camera sweeps the map at zoom 1 and fully zoomed out, and prints each phase's
//! frame times by span (see `bb_render::frame_timing`), the GPU's included (each frame
//! waits for it).
//!
//! ```text
//! cargo run --release -p bb-render --example world_pan -- [WIDTHxHEIGHT] [FRAMES] [--mods] [--shot OUT.png] [--battle]
//! ```
//!
//! `BB_PAN_UNKEPT=1` draws the world as a scene of the tiles on screen each frame, as
//! before the map was kept on the GPU (`WorldRenderer::draw`).
//!
//! `--battle` pans a battle's map instead (the battle `render_battle` starts), at zoom 1
//! and fully zoomed out, fog of war off.
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
	let battle = take_flag(&mut args, "--battle");
	let running = take_flag(&mut args, "--run");
	let shot = take_value(&mut args, "--shot");
	let compare = take_flag(&mut args, "--compare");
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
	if battle {
		start_battle(&engine);
	}
	if running {
		// The world running at 4x (no autosaves), its details turning, stepping and
		// fading, parties moving.
		harness::unpause_world(&engine).expect("unpause");
		engine
			.run_script(
				"::World.State.m.IsAutosaving = false; local p = ::World.State.getPlayer(); p.m.IsAttackable = false; p.m.IsAttackableByAI = false; ::World.setSpeedMult(4.0);",
				"run",
			)
			.expect("run");
	}
	let mut archives = harness::data_archives(dir).unwrap();
	if mods {
		archives.extend(harness::mod_archives(dir).unwrap());
	}
	let gpu = Gpu::headless().expect("gpu");
	let mut renderer = WorldRenderer::new(&gpu, Arc::new(GameFiles::open(archives).expect("files")), 8);
	let target = RenderTarget::new(&gpu, width, height);
	// `--compare`: a second renderer drawing the scene of the tiles on screen each frame.
	let mut twin = compare.then(|| WorldRenderer::new(&gpu, Arc::new(GameFiles::open(harness::data_archives(dir).unwrap()).expect("files")), 8));
	let center = if battle {
		let camera = engine.tactical_scene().camera;
		(camera.pos.0 as f32, camera.pos.1 as f32)
	} else {
		engine.world_camera().pos
	};
	let camera = if battle { "::Tactical.getCamera()" } else { "::World.getCamera()" };
	frame_timing::enable();

	let phases = if battle {
		[("zoom 1", 1.0f32, 600.0f32), ("zoomed out", 100.0, 1200.0)]
	} else {
		[("zoom 1", 1.0f32, 1500.0f32), ("zoomed out", 100.0, 6000.0)]
	};
	for (name, zoom, reach) in phases {
		engine.run_script(&format!("{camera}.Zoom = {zoom:?};"), "zoom").expect("zoom");
		let mut spans: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
		let mut quads = Vec::new();
		let (mut changes, mut revisions) = (Vec::new(), Vec::new());
		for i in 0..frames {
			let t = i as f32;
			let x = center.0 + reach * (std::f32::consts::TAU * t / 240.0).sin();
			let y = center.1 + reach * 0.5 * (std::f32::consts::TAU * t / 360.0).cos();
			engine
				.run_script(&format!("{camera}.setPos(::createVec({x:?}, {y:?}));"), "pan")
				.expect("pan");
			let started = Instant::now();
			{
				let _span = frame_timing::span("step");
				if running {
					harness::run_frame(&engine, 16.0).expect("frame");
				} else {
					engine.frame(16.0).expect("frame");
				}
			}
			if running && i % 60 == 0 {
				// Kept running through menus and pauses, as a player would.
				engine
					.run_script("if (::World.State.isPaused()) { ::World.State.setAutoPause(false); ::World.State.setPause(false); } ::World.setSpeedMult(4.0);", "run")
					.expect("run");
			}
			let stats = if battle {
				draw_battle(&engine, &gpu, &mut renderer, &target, Atlases::DrawReady)
			} else {
				draw(&engine, &gpu, &mut renderer, twin.as_mut(), &target, Atlases::DrawReady)
			};
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
			let (changed, revision) = MAP.with(|m| m.get());
			changes.push(changed as f64);
			revisions.push(revision as f64);
		}
		let zoom_now = if battle { engine.tactical_scene().camera.zoom as f32 } else { engine.world_camera().zoom };
		println!("== {name} (camera zoom {zoom_now:.2}), {width}x{height}, {} frames", frames.saturating_sub(60));
		print_row("quads", &mut quads);
		println!("  virtual time   {:.1} s", engine.time().virtual_seconds());
		if !battle {
			print_row("changed", &mut changes);
			println!("  map revisions  {} (from {} to {})", revisions.last().unwrap_or(&0.0) - revisions.first().unwrap_or(&0.0), revisions.first().unwrap_or(&0.0), revisions.last().unwrap_or(&0.0));
		}
		for (span, times) in &mut spans {
			print_row(span, times);
		}
	}

	if let Some(twin) = twin.as_mut() {
		compare_views(&engine, &gpu, &mut renderer, twin, &target, center);
	}
	if let Some(path) = shot {
		engine.run_script(&format!("{camera}.Zoom = 100.0;"), "zoom").expect("zoom");
		engine
			.run_script(&format!("{camera}.setPos(::createVec({:?}, {:?}));", center.0, center.1), "pan")
			.expect("pan");
		engine.frame(16.0).expect("frame");
		if battle {
			draw_battle(&engine, &gpu, &mut renderer, &target, Atlases::WaitForAll);
		} else {
			draw(&engine, &gpu, &mut renderer, twin.as_mut(), &target, Atlases::WaitForAll);
		}
		save_png(&path, &target.read(&gpu).expect("read back")).expect("save");
		eprintln!("wrote {path}");
	}
}

/// One frame of the world as `bb` draws it.
fn draw(
	engine: &harness::GameEngine,
	gpu: &Gpu,
	renderer: &mut WorldRenderer,
	twin: Option<&mut WorldRenderer>,
	target: &RenderTarget,
	atlases: Atlases,
) -> bb_render::FrameStats {
	renderer.begin_frame();
	let hints = engine.sprites().take_hints();
	engine.sprites().with(|registry| renderer.prefetch(registry, hints));
	if let Some(patch) = engine.world_grid().take_fog_patch() {
		renderer.update_fog(gpu, &patch);
		if let Some(twin) = twin {
			twin.update_fog(gpu, &patch);
		}
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
	if std::env::var_os("BB_PAN_UNKEPT").is_some() {
		let scene = {
			let _span = frame_timing::span("world scene");
			engine.world_scene_where(|pos| view.shows(pos))
		};
		let _span = frame_timing::span("world draw");
		return engine.sprites().with(|registry| renderer.draw(gpu, target, &scene, registry, &view, atlases));
	}
	let (scene, changes, edits) = {
		let _span = frame_timing::span("world scene");
		let grid = engine.world_grid();
		(engine.world_scene_where(|_| false), grid.take_detail_changes(), grid.take_map_edits())
	};
	let grid = engine.world_grid();
	MAP.with(|m| m.set((changes.len(), grid.map_revision())));
	if std::env::var_os("BB_PAN_LIST_CHANGES").is_some() && !changes.is_empty() {
		let mut by: BTreeMap<String, usize> = BTreeMap::new();
		for c in &changes {
			*by.entry(format!("{} rot {:.0} frame {} alpha {} hidden {}", c.brush, c.rotation, c.frame, c.alpha, c.hidden)).or_default() += 1;
		}
		eprintln!("changes: {:?}", by.iter().take(12).collect::<Vec<_>>());
	}
	let tiles = || grid.scene_map();
	let map = bb_render::WorldMap {
		revision: grid.map_generation(),
		tiles: &tiles,
		edits: &edits,
		changes: &changes,
	};
	let _span = frame_timing::span("world draw");
	engine.sprites().with(|registry| renderer.draw_kept(gpu, target, &scene, &map, registry, &view, atlases))
}

/// The battle `render_battle` shoots, up to the player's first turn, its camera on the
/// map's centre and fog of war off.
fn start_battle(engine: &harness::GameEngine) {
	let frame = |engine: &harness::GameEngine| {
		engine.frame(1000.0 / 60.0).expect("frame");
		engine.pump_ui().expect("ui");
		engine.run_script("::__pilot();", "pilot").expect("pilot");
	};
	engine.run_script(include_str!("battle_setup.nut"), "setup").expect("setup");
	for _ in 0..120 {
		frame(engine);
	}
	engine.run_script("::__startBattle();", "battle").expect("battle");
	for _ in 0..6000 {
		frame(engine);
		engine.run_script("::__turn <- ::__playerTurn();", "turn").expect("turn");
		if squirrel::SqTable::root(engine.vm()).get::<bool>("__turn") == Some(true) {
			break;
		}
	}
	engine.run_script("::__battleCamera(); ::__fogOff();", "camera").expect("camera");
	for _ in 0..10 {
		frame(engine);
	}
}

thread_local! {
	/// The last frame's details changed and the map's revision.
	static MAP: std::cell::Cell<(usize, u64)> = const { std::cell::Cell::new((0, 0)) };
}

/// One frame of a battle as `bb` draws it.
fn draw_battle(
	engine: &harness::GameEngine,
	gpu: &Gpu,
	renderer: &mut WorldRenderer,
	target: &RenderTarget,
	atlases: Atlases,
) -> bb_render::FrameStats {
	renderer.begin_frame();
	let hints = engine.sprites().take_hints();
	engine.sprites().with(|registry| renderer.prefetch(registry, hints));
	let scene = {
		let _span = frame_timing::span("battle scene");
		engine.tactical_scene()
	};
	let view = WorldView {
		center: [scene.camera.pos.0 as f32, scene.camera.pos.1 as f32],
		zoom: 1.0 / scene.camera.zoom as f32,
		width: target.width,
		height: target.height,
	};
	let _span = frame_timing::span("battle draw");
	engine.sprites().with(|registry| renderer.draw_tactical(gpu, target, &scene, registry, &view, atlases))
}

/// Draw the world at a few views with the map kept (`renderer`, as it was kept while
/// panning) and as a scene of the tiles on screen (`twin`), at the same clock, and print
/// how many pixels differ.
fn compare_views(engine: &harness::GameEngine, gpu: &Gpu, renderer: &mut WorldRenderer, twin: &mut WorldRenderer, target: &RenderTarget, center: (f32, f32)) {
	let other = RenderTarget::new(gpu, target.width, target.height);
	let mut differing = 0;
	for (zoom, dx, dy) in [(100.0f32, 0.0f32, 0.0f32), (1.0, 0.0, 0.0), (1.0, 1400.0, 600.0), (1.0, -2500.0, -900.0), (3.0, 3000.0, 1000.0)] {
		engine
			.run_script(
				&format!("::World.getCamera().Zoom = {zoom:?}; ::World.getCamera().setPos(::createVec({:?}, {:?}));", center.0 + dx, center.1 + dy),
				"view",
			)
			.expect("view");
		engine.frame(16.0).expect("frame");
		draw(engine, gpu, renderer, Some(&mut *twin), target, Atlases::WaitForAll);
		let clock = Clock {
			real: engine.time().real_seconds() as f32,
			virtual_time: engine.time().virtual_seconds() as f32,
		};
		twin.begin_frame();
		twin.set_clock(clock);
		let camera = engine.world_camera();
		let view = WorldView {
			center: [camera.pos.0, camera.pos.1],
			zoom: 1.0 / camera.zoom,
			width: target.width,
			height: target.height,
		};
		let scene = engine.world_scene_where(|pos| view.shows(pos));
		engine.sprites().with(|registry| twin.draw(gpu, &other, &scene, registry, &view, Atlases::WaitForAll));
		// The first frame drawn after the panning one differs from it and from those after
		// it, the same way with the map drawn as a scene throughout (`BB_PAN_UNKEPT`): so
		// the scene is drawn twice, and the second compared.
		let first = other.read(gpu).expect("read");
		renderer.begin_frame();
		engine.sprites().with(|registry| renderer.draw(gpu, &other, &scene, registry, &view, Atlases::WaitForAll));
		let b = other.read(gpu).expect("read");
		let unsteady = first.rgba.chunks(4).zip(b.rgba.chunks(4)).filter(|(x, y)| x != y).count();
		let a = target.read(gpu).expect("read");
		let diff = a.rgba.chunks(4).zip(b.rgba.chunks(4)).filter(|(x, y)| x != y).count();
		println!("compare: zoom {:.2} at {:?}: {diff} pixels differ ({unsteady} between the scene's two draws)", camera.zoom, camera.pos);
		if diff > 0 && let Ok(dir) = std::env::var("BB_PAN_COMPARE_DIR") {
			save_png(format!("{dir}/kept_{zoom}_{dx}.png"), &a).expect("save");
			save_png(format!("{dir}/scene_{zoom}_{dx}.png"), &b).expect("save");
		}
		differing += diff;
	}
	println!("compare: {differing} pixels differ in all");
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
