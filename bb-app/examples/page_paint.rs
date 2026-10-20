//! What the UI page's paints cost, and how often it paints, with the game's frames paced
//! at a display's refresh rate: the game as `bb` runs it, offscreen, at the main menu.
//!
//! ```text
//! cargo run --release -p bb-app --features ui --example page_paint -- [WIDTH HEIGHT [HZ [SECONDS]]]
//! ```
//!
//! Defaults: 2560x1358 at 180 Hz, 5 s a scene. Each scene runs on the main menu, settled:
//! - `idle`: nothing moves;
//! - `css small`: a 64 px square spun by a CSS animation (the compositor's);
//! - `velocity`: a 64 px square moved by Velocity (the page's script, a tick per page
//!   frame: its tick rate is the page's animation frame rate);
//! - `css full`: the whole view faded in and out by a CSS animation (every pixel changes
//!   every page frame: the worst case).
//!
//! Per scene: the game's frame rate, the page's paints a second, the time our paint
//! handler takes (importing the paint's GPU buffer, copying what changed and waiting for
//! it), the pixels copied, the game's draw of the page ("ui page" span), each process's
//! CPU (the game's, by its busiest threads, and the page's renderer), and memory (PSS).
//! Then what "is the cursor over the UI" costs (`alpha_at`, read back from the GPU a
//! tile at a time): a mouse sweeping a still page, and one moving every frame over the
//! whole view fading (every call reads back).

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_app::game::{Boot, Game, Start, Store};
use bb_engine::harness::STEAM_DATA_DIR;
use bb_engine::tactical_player as player;
use bb_render::{Gpu, RenderTarget};
use squirrel_bb::VideoMode;

const AT_MENU: &str = "::RootState.get(\"MainMenuState\") != null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible() && !::RootState.get(\"MainMenuState\").m.MainMenuScreen.isAnimating()";

const CLEAR: &str = r#"(function () {
	var old = document.getElementById('pp-scene'); if (old) old.parentNode.removeChild(old);
	if (window.jQuery && jQuery.Velocity && window.__ppBox) { jQuery(window.__ppBox).velocity('stop', true); }
	window.__ppBox = null; window.__ppTicks = 0;
	return true;
})()"#;

const CSS_SMALL: &str = r#"(function () {
	var scene = document.createElement('div'); scene.id = 'pp-scene';
	scene.innerHTML = '<style>@keyframes pp-spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }</style>'
		+ '<div style="position:absolute;left:40px;top:40px;width:64px;height:64px;background:#c33;z-index:99999;animation:pp-spin 1s linear infinite"></div>';
	document.body.appendChild(scene);
	return true;
})()"#;

const VELOCITY: &str = r#"(function () {
	var scene = document.createElement('div'); scene.id = 'pp-scene';
	scene.innerHTML = '<div style="position:absolute;left:40px;top:40px;width:64px;height:64px;background:#3c3;z-index:99999"></div>';
	document.body.appendChild(scene);
	var box = scene.firstChild; window.__ppBox = box; window.__ppTicks = 0;
	function go() {
		jQuery(box).velocity({ left: 600 }, { duration: 1000, easing: 'linear', progress: function () { window.__ppTicks++; } })
			.velocity({ left: 40 }, { duration: 1000, easing: 'linear', progress: function () { window.__ppTicks++; }, complete: function () { if (window.__ppBox === box) go(); } });
	}
	go();
	return !!(window.jQuery && jQuery.Velocity);
})()"#;

const CSS_FULL: &str = r#"(function () {
	var scene = document.createElement('div'); scene.id = 'pp-scene';
	scene.innerHTML = '<style>@keyframes pp-fade { from { opacity: 0.1; } to { opacity: 0.9; } }</style>'
		+ '<div style="position:absolute;left:0;top:0;right:0;bottom:0;background:#246;z-index:99999;animation:pp-fade 1s linear infinite alternate"></div>';
	document.body.appendChild(scene);
	return true;
})()"#;

/// A one-second Velocity animation and a one-second CSS transition: how long each took by
/// the page's clock (they must keep real time at any frame rate).
const TIMED: &str = r#"(function () {
	var scene = document.createElement('div'); scene.id = 'pp-scene';
	scene.innerHTML = '<div style="position:absolute;left:40px;top:140px;width:64px;height:64px;background:#33c;z-index:99999"></div>'
		+ '<div style="position:absolute;left:40px;top:240px;width:64px;height:64px;background:#cc3;z-index:99999;transition:left 1s linear"></div>';
	document.body.appendChild(scene);
	var start = performance.now();
	window.__ppVelocity = null; window.__ppTransition = null;
	jQuery(scene.firstChild).velocity({ left: 600 }, { duration: 1000, easing: 'linear', complete: function () { window.__ppVelocity = performance.now() - start; } });
	var moving = scene.lastChild;
	moving.addEventListener('transitionend', function () { window.__ppTransition = performance.now() - start; });
	moving.getBoundingClientRect();
	moving.style.left = '600px';
	return true;
})()"#;

struct Run<'a> {
	game: Game,
	gpu: &'a Gpu,
	target: RenderTarget,
	interval: Duration,
	next: Instant,
	/// The page's paint count at the last draw, and how many frames drew 0, 1, 2 and
	/// more new paints.
	paints: u64,
	cadence: [u64; 4],
}

impl Run<'_> {
	fn holds(&self, cond: &str) -> bool {
		player::eval_i64(self.game.front().engine(), &format!("({cond}) ? 1 : 0")).ok().flatten() == Some(1)
	}

	/// One frame, paced: step by the interval, draw, and wait for the next frame's time.
	fn frame(&mut self) -> Result<(), String> {
		let ms = self.interval.as_secs_f64() * 1000.0;
		self.game.step(ms)?;
		self.game.draw(self.gpu, &self.target);
		let paints = self.game.front().session().host().paints();
		self.cadence[(paints - self.paints).min(3) as usize] += 1;
		self.paints = paints;
		self.gpu.device.poll(wgpu::PollType::Poll).map_err(|e| e.to_string())?;
		self.next += self.interval;
		let now = Instant::now();
		match self.next.checked_duration_since(now) {
			Some(wait) => std::thread::sleep(wait),
			// Behind: start again from now rather than rush to catch up.
			None => self.next = now,
		}
		Ok(())
	}

	fn eval(&self, code: &str) -> Result<bb_ui::json::Json, String> {
		self.game.front().session().eval(code)
	}
}

/// CPU time (user + system) of `pid`, or of one of its threads.
fn cpu_of(stat_path: &str) -> Option<(String, Duration)> {
	let stat = std::fs::read_to_string(stat_path).ok()?;
	let (head, tail) = stat.rsplit_once(')')?;
	let name = head.split_once('(')?.1.to_string();
	let fields: Vec<&str> = tail.split_whitespace().collect();
	let ticks = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
	Some((name, Duration::from_millis(ticks * 10)))
}

/// CPU time by thread name in this process, and by role in each child.
fn cpu_now(children: &[(u32, String)]) -> HashMap<String, Duration> {
	let mut cpu = HashMap::new();
	if let Ok(tasks) = std::fs::read_dir("/proc/self/task") {
		for task in tasks.flatten() {
			if let Some((name, time)) = cpu_of(&task.path().join("stat").to_string_lossy()) {
				*cpu.entry(format!("game: {name}")).or_default() += time;
			}
		}
	}
	for (pid, role) in children {
		if let Some((_, time)) = cpu_of(&format!("/proc/{pid}/stat")) {
			*cpu.entry(role.clone()).or_default() += time;
		}
	}
	cpu
}

fn scene(run: &mut Run, name: &str, setup: &str, seconds: f64, children: &[(u32, String)]) -> Result<(), String> {
	run.eval(CLEAR)?;
	if !setup.is_empty() {
		let ok = run.eval(setup)?;
		if ok != bb_ui::json::Json::Bool(true) {
			return Err(format!("{name}: the scene didn't set up ({ok:?})"));
		}
	}
	// Settle into it.
	let warm = Instant::now();
	while warm.elapsed() < Duration::from_millis(500) {
		run.frame()?;
	}
	let host = run.game.front().session().host();
	let (paints, copies, ticks) = (host.paint_stats(), run.game.front().copy_stats(), run.eval("window.__ppTicks || 0")?);
	let cpu = cpu_now(children);
	bb_render::frame_timing::take();
	run.cadence = [0; 4];
	let start = Instant::now();
	let mut frames = 0u64;
	while start.elapsed().as_secs_f64() < seconds {
		run.frame()?;
		frames += 1;
	}
	let wall = start.elapsed().as_secs_f64();
	let spans = bb_render::frame_timing::take();
	let host = run.game.front().session().host();
	let paints = host.paint_stats().since(&paints);
	let copies = run.game.front().copy_stats().since(&copies);
	let ticks = run.eval("window.__ppTicks || 0")?.as_i64().unwrap_or(0) as f64 - ticks.as_i64().unwrap_or(0) as f64;
	let cpu_after = cpu_now(children);
	let per_frame = |v: f64| v / frames as f64;
	let mib = |b: u64| b as f64 / (1024.0 * 1024.0);
	println!("== {name}: {frames} frames in {wall:.2} s = {:.1} fps", frames as f64 / wall);
	let per_paint = |t: Duration| if paints.paints > 0 { t.as_secs_f64() * 1000.0 / paints.paints as f64 } else { 0.0 };
	println!(
		"  page: {} paints = {:.1}/s ({} failed); handler {:.3} ms a paint; damage {:.0} px a paint",
		paints.paints,
		paints.paints as f64 / wall,
		paints.failures,
		per_paint(paints.handler),
		if paints.paints > 0 { paints.damaged_pixels as f64 / paints.paints as f64 } else { 0.0 },
	);
	let [none, one, two, more] = run.cadence;
	println!("  frames that drew 0 / 1 / 2 / 3+ new paints: {none} / {one} / {two} / {more}");
	if ticks > 0.0 {
		println!("  page script ticks (Velocity progress): {:.1}/s", ticks / wall);
	}
	println!(
		"  copy (GPU to GPU): {} paints in {} rects, {:.3} MiB a frame; import {:.3} ms, copy + wait {:.3} ms a paint",
		copies.paints,
		copies.rects,
		per_frame(mib(copies.pixels * 4)),
		if copies.paints > 0 { copies.import.as_secs_f64() * 1000.0 / copies.paints as f64 } else { 0.0 },
		if copies.paints > 0 { copies.copy.as_secs_f64() * 1000.0 / copies.paints as f64 } else { 0.0 },
	);
	let span = |n: &str| spans.iter().find(|(s, _)| *s == n).map_or(0.0, |(_, t)| per_frame(t.as_secs_f64() * 1000.0));
	println!("  frame: step {:.3} ms, ui page draw {:.3} ms", span("step"), span("ui page"));
	if let Some((used, reserved)) = run.gpu.allocator_use() {
		println!("  wgpu's allocator: {:.1} MiB used in {:.1} MiB of blocks", mib(used), mib(reserved));
	}
	let mut used: Vec<(String, f64)> = cpu_after
		.iter()
		.map(|(k, t)| (k.clone(), t.saturating_sub(cpu.get(k).copied().unwrap_or_default()).as_secs_f64()))
		.filter(|(_, t)| *t > 0.0)
		.collect();
	used.sort_by(|a, b| b.1.total_cmp(&a.1));
	let game: f64 = used.iter().filter(|(k, _)| k.starts_with("game: ")).map(|(_, t)| t).sum();
	let rest: f64 = used.iter().filter(|(k, _)| !k.starts_with("game: ")).map(|(_, t)| t).sum();
	println!(
		"  CPU: game process {:.0}% of a core ({:.3} ms a frame), CEF children {:.0}% ({:.3} ms a frame)",
		game / wall * 100.0,
		per_frame(game * 1000.0),
		rest / wall * 100.0,
		per_frame(rest * 1000.0)
	);
	let top: Vec<String> = used.iter().take(8).map(|(k, t)| format!("{k} {:.0}%", t / wall * 100.0)).collect();
	println!("    {}", top.join(", "));
	Ok(())
}

/// What `alpha_at` (is the cursor over the UI) costs: a mouse sweeping the still page
/// (tiles read once, then kept), and a mouse moving every frame over the whole view
/// fading (each paint changes every tile: every call reads back).
fn alpha_cost(run: &mut Run) -> Result<(), String> {
	let (w, h) = run.game.screen();
	// Moves `from..from + calls` of a diagonal sweep, 7 px a move: the time in
	// `alpha_at`, the tiles read back, and the moves over the UI.
	let sweep = |run: &Run, from: u32, calls: u32| -> (Duration, u64, u32) {
		let host = run.game.front().session().host();
		let reads = run.game.front().copy_stats().reads;
		let mut time = Duration::ZERO;
		let mut over = 0;
		for i in from..from + calls {
			let (x, y) = ((i * 7) % w, (i * 7 * h / w.max(1)) % h);
			let started = Instant::now();
			over += u32::from(host.alpha_at(x as i32, y as i32) > 0);
			time += started.elapsed();
		}
		(time, run.game.front().copy_stats().reads - reads, over)
	};
	run.eval(CLEAR)?;
	for _ in 0..30 {
		run.frame()?;
	}
	let (time, reads, over) = sweep(run, 0, 2000);
	println!(
		"== alpha_at, still page: {:.1} µs a call over 2000 moves ({reads} tiles read back, {over} over the UI)",
		time.as_secs_f64() * 1e6 / 2000.0
	);
	run.eval(CSS_FULL)?;
	for _ in 0..30 {
		run.frame()?;
	}
	let (mut time, mut reads, mut calls) = (Duration::ZERO, 0, 0u32);
	let started = Instant::now();
	while started.elapsed() < Duration::from_secs(2) {
		run.frame()?;
		let (t, r, _) = sweep(run, calls, 1);
		time += t;
		reads += r;
		calls += 1;
	}
	println!(
		"== alpha_at, whole view fading, a move a frame: {:.1} µs a call ({calls} calls, {reads} tiles read back)",
		time.as_secs_f64() * 1e6 / f64::from(calls.max(1))
	);
	Ok(())
}

fn run(width: i64, height: i64, hz: f64, seconds: f64) -> Result<(), String> {
	let gpu = Gpu::headless_with(bb_app::front::GPU_FEATURES)?;
	let boot = Boot {
		data_dir: Path::new(STEAM_DATA_DIR),
		start: Start::Menu,
		mods: false,
		sound: false,
		world_size: None,
		mode: VideoMode { width, height, refresh_rate: hz.round() as i64, ..VideoMode::DEFAULT },
		settings: None,
		devtools: None,
		saves: None,
		cef_log: None,
		store: Store::default(),
	};
	let game = Game::start(&gpu, &boot)?;
	let (w, h) = game.screen();
	let target = RenderTarget::new(&gpu, w, h);
	let interval = Duration::from_secs_f64(1.0 / hz);
	let mut run = Run { game, gpu: &gpu, target, interval, next: Instant::now(), paints: 0, cadence: [0; 4] };
	bb_render::frame_timing::enable();
	let since = Instant::now();
	while !run.holds(AT_MENU) {
		run.frame()?;
		if since.elapsed() > Duration::from_secs(60) {
			return Err("the main menu never showed".into());
		}
	}
	let settle = Instant::now();
	while settle.elapsed() < Duration::from_secs(1) {
		run.frame()?;
	}
	let children: Vec<(u32, String)> = bb_app::stats::tree_memory()
		.unwrap_or_default()
		.into_iter()
		.filter(|p| p.role != "game")
		.map(|p| (p.pid, p.role))
		.collect();
	println!("{w}x{h} at {hz} Hz, {seconds} s a scene");
	for (name, setup) in [("idle", ""), ("css small", CSS_SMALL), ("velocity", VELOCITY), ("css full", CSS_FULL), ("idle again", "")] {
		scene(&mut run, name, setup, seconds, &children)?;
	}
	alpha_cost(&mut run)?;
	run.eval(CLEAR)?;
	run.eval(TIMED)?;
	let timed = Instant::now();
	while timed.elapsed() < Duration::from_millis(1500) {
		run.frame()?;
	}
	println!(
		"== timing: a 1000 ms Velocity animation took {} ms, a 1000 ms CSS transition {} ms (page clock)",
		run.eval("Math.round(window.__ppVelocity)")?.to_text(),
		run.eval("Math.round(window.__ppTransition)")?.to_text()
	);
	run.eval(CLEAR)?;
	let memory = bb_app::stats::tree_memory().unwrap_or_default();
	let pss: u64 = memory.iter().map(|p| p.pss()).sum();
	let parts: Vec<String> = memory.iter().map(|p| format!("{} {:.0}", p.role, p.pss() as f64 / 1048576.0)).collect();
	println!("memory (PSS): {:.0} MB in all: {}", pss as f64 / 1048576.0, parts.join(", "));
	println!("page texture on the GPU: {:.1} MiB", run.game.front().layer_bytes() as f64 / 1048576.0);
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	bb_app::platform::tune_allocator();
	let args: Vec<String> = std::env::args().skip(1).collect();
	let num = |i: usize, default: f64| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(default);
	let (width, height, hz, seconds) = (num(0, 2560.0) as i64, num(1, 1358.0) as i64, num(2, 180.0), num(3, 5.0));
	match run(width, height, hz, seconds) {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		}
	}
}
