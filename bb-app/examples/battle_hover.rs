//! What a player's mouse over a battle costs the frames: the game as `bb` runs it,
//! offscreen with the real UI page, the install's mods mounted (unless `--no-mods`), in a
//! new campaign's battle on a brother's turn, the pointer swept back and forth over the
//! battlefield (units and tiles, so their tooltips build) for `--seconds`, paced at
//! `--hz`. Between frames the mouse moves `--moves` times, spread over the wait, as a
//! 1000 Hz mouse's system events come.
//!
//! ```text
//! cargo run --profile test-release -p bb-app --features ui --example battle_hover -- \
//!     [--no-mods] [--seconds 30] [--hz 180] [--moves 6] [--seed e2e] [--end-turns SECS] [--frames FILE]
//! ```
//!
//! `--end-turns SECS` ends the brother's turn that often, so the enemies' turns run
//! (and the page's churn with them) while the pointer sweeps.
//!
//! Reports the frame intervals (percentiles, the 1% and 0.1% lows, those over twice the
//! budget), the spans that make up the frames' time (`page park`, the input path, the
//! scripts' calls into the page), the waits for the page's own tasks, and the page's
//! renderer's memory (PSS, and its V8 heap) before
//! and after the sweep. `BB_UI_V8_FLAGS` sets the page's V8 flags for an A/B.
//! `--frames FILE` writes each frame's spans as `BB_FRAME_TIMES` does.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_app::game::{Boot, Game, Start, Store};
use bb_app::stats::{self, MappingKind};
use bb_engine::harness::STEAM_DATA_DIR;
use bb_engine::tactical_player as player;
use bb_render::{Gpu, RenderTarget, frame_timing};
use squirrel_bb::VideoMode;

struct Options {
	mods: bool,
	seconds: f64,
	hz: f64,
	moves: u32,
	seed: String,
	frames: Option<PathBuf>,
	/// End the brother's turn this often (seconds), so the AI's turns run too.
	end_turns: Option<f64>,
}

fn options() -> Result<Options, String> {
	let mut o = Options { mods: true, seconds: 30.0, hz: 180.0, moves: 6, seed: "e2e".into(), frames: None, end_turns: None };
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().ok_or(format!("{arg} needs a value"));
		match arg.as_str() {
			"--no-mods" => o.mods = false,
			"--seconds" => o.seconds = value()?.parse().map_err(|e| format!("--seconds: {e}"))?,
			"--hz" => o.hz = value()?.parse().map_err(|e| format!("--hz: {e}"))?,
			"--moves" => o.moves = value()?.parse().map_err(|e| format!("--moves: {e}"))?,
			"--seed" => o.seed = value()?,
			"--frames" => o.frames = Some(PathBuf::from(value()?)),
			"--end-turns" => o.end_turns = Some(value()?.parse().map_err(|e| format!("--end-turns: {e}"))?),
			other => return Err(format!("unknown argument {other}")),
		}
	}
	Ok(o)
}

/// The battle: the intro event answered, the company quick so a brother takes the first
/// turn, against bandits (as `bb-ui`'s `mods_battle_ui`).
const BATTLE: &str = r#"
	::World.State.m.IsAutosaving = false;
	local screen = ::World.State.getEventScreen();
	if (screen != null && screen.isVisible()) screen.onButtonPressed(0);
	foreach (bro in ::World.getPlayerRoster().getAll()) bro.getBaseProperties().Initiative = 1000;
	local st = ::World.State;
	local p = st.getLocalCombatProperties(st.getPlayer().getPos());
	p.CombatID = "Event";
	p.IsAutoAssigningBases = false;
	p.Entities = [];
	::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditDefenders, 100, ::Const.Faction.Enemy);
	st.startScriptedCombat(p, false, false, true);
"#;

const IN_WORLD: &str = "\"State\" in ::World && ::World.State != null && ::World.State.getPlayer() != null \
	&& !::World.State.isInLoadingScreen() && !::LoadingScreen.isVisible()";
const INTRO_SHOWN: &str =
	"(function() { local s = ::World.State.getEventScreen(); return s != null && s.isVisible() && !s.isAnimating(); })()";
const IN_BATTLE: &str = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible()";
const BROTHERS_TURN: &str = "(function() { local a = ::Tactical.TurnSequenceBar.getActiveEntity(); \
	return a != null && a.isPlayerControlled(); })()";

struct Run<'a> {
	game: Game,
	gpu: &'a Gpu,
	target: RenderTarget,
	interval: Duration,
}

impl Run<'_> {
	fn holds(&self, cond: &str) -> bool {
		player::eval_i64(self.game.front().engine(), &format!("({cond}) ? 1 : 0")).ok().flatten() == Some(1)
	}

	/// One frame as the window runs it: step, draw, present.
	fn frame(&mut self, ms: f64) -> Result<(), String> {
		self.game.step(ms)?;
		if self.game.draw_if_changed(self.gpu, &self.target).is_some() {
			self.game.presented();
		}
		self.gpu.device.poll(wgpu::PollType::Poll).map_err(|e| e.to_string())?;
		Ok(())
	}

	/// Frames at the pace until `cond` holds (at most `limit`).
	fn until(&mut self, cond: &str, limit: Duration, what: &str) -> Result<(), String> {
		let since = Instant::now();
		while !self.holds(cond) {
			let start = Instant::now();
			self.frame(self.interval.as_secs_f64() * 1000.0)?;
			if since.elapsed() > limit {
				return Err(format!("{what}: not after {limit:?}"));
			}
			if let Some(wait) = self.interval.checked_sub(start.elapsed()) {
				std::thread::sleep(wait);
			}
		}
		Ok(())
	}
}

/// The pointer's path: back and forth over the middle of the screen, row by row.
struct Sweep {
	width: f64,
	height: f64,
	/// Distance travelled, in pixels.
	at: f64,
}

impl Sweep {
	fn next(&mut self, step: f64) -> (i64, i64) {
		self.at += step;
		let (left, right) = (self.width * 0.12, self.width * 0.88);
		let (top, bottom) = (self.height * 0.2, self.height * 0.8);
		let row_len = right - left;
		let row = (self.at / row_len) as i64;
		let along = self.at % row_len;
		let x = if row % 2 == 0 { left + along } else { right - along };
		let rows = ((bottom - top) / 37.0) as i64;
		let r = row % (2 * rows);
		let y = top + 37.0 * (if r < rows { r } else { 2 * rows - r }) as f64;
		(x as i64, y as i64)
	}
}

/// A frame's line in the report: its interval, its work, its spans and counters.
struct Frame {
	interval: f64,
	work: f64,
	spans: Vec<(&'static str, f64)>,
	counts: Vec<(&'static str, u64)>,
}

impl Frame {
	fn span(&self, name: &str) -> f64 {
		self.spans.iter().filter(|(n, _)| *n == name).map(|(_, t)| t).sum()
	}
}

fn pct(values: &mut [f64], q: f64) -> f64 {
	if values.is_empty() {
		return 0.0;
	}
	values.sort_by(|a, b| a.partial_cmp(b).unwrap());
	values[((values.len() - 1) as f64 * q).round() as usize]
}

/// The renderer's PSS and its V8 heap, in MB.
fn renderer_memory() -> (f64, f64) {
	let mb = |b: u64| b as f64 / 1048576.0;
	let processes = stats::tree_memory().unwrap_or_default();
	let renderers = processes.iter().filter(|p| p.role.starts_with("renderer"));
	let (mut pss, mut v8) = (0, 0);
	for p in renderers {
		pss += p.pss();
		v8 += p.kinds.iter().filter(|(k, _)| *k == MappingKind::V8).map(|(_, b)| b).sum::<u64>();
	}
	(mb(pss), mb(v8))
}

fn run(o: &Options) -> Result<(), String> {
	let gpu = Gpu::headless_with(bb_app::front::GPU_FEATURES)?;
	let boot = Boot {
		data_dir: Path::new(STEAM_DATA_DIR),
		start: Start::Menu,
		mods: o.mods,
		sound: false,
		world_size: None,
		mode: VideoMode { width: 2560, height: 1440, refresh_rate: o.hz.round() as i64, ..VideoMode::DEFAULT },
		settings: None,
		devtools: None,
		saves: None,
		cef_log: None,
		store: Store::default(),
		update_check: None,
		display: None,
	};
	let game = Game::start(&gpu, &boot)?;
	let (w, h) = game.screen();
	let target = RenderTarget::new(&gpu, w, h);
	let interval = Duration::from_secs_f64(1.0 / o.hz);
	let mut run = Run { game, gpu: &gpu, target, interval };
	run.game.front().session().start_campaign(&o.seed)?;
	let since = Instant::now();
	let mut pressed_save = false;
	while !run.holds(IN_WORLD) {
		if !pressed_save && run.holds("(\"MSU\" in getroottable()) && ::MSU.SettingsScreen.isVisible()") {
			run.game.run_script("::MSU.SettingsScreen.onSaveButtonPressed({});")?;
			pressed_save = true;
		}
		run.frame(interval.as_secs_f64() * 1000.0)?;
		if since.elapsed() > Duration::from_secs(300) {
			return Err("the world never came up".into());
		}
	}
	run.until(INTRO_SHOWN, Duration::from_secs(60), "the intro event")?;
	run.game.run_script(BATTLE)?;
	run.until(IN_BATTLE, Duration::from_secs(120), "the battle")?;
	run.until(BROTHERS_TURN, Duration::from_secs(60), "a brother's turn")?;
	// Settle: the battle's start (its music, the camera's glide, the first pictures).
	for _ in 0..(o.hz as usize * 3) {
		let start = Instant::now();
		run.frame(interval.as_secs_f64() * 1000.0)?;
		if let Some(wait) = interval.checked_sub(start.elapsed()) {
			std::thread::sleep(wait);
		}
	}
	let before = renderer_memory();
	let rested = run.game.front().page_rested();

	frame_timing::enable();
	let _ = frame_timing::take();
	let _ = frame_timing::take_counts();
	let mut sweep = Sweep { width: f64::from(w), height: f64::from(h), at: 0.0 };
	// A 1000 Hz mouse moving 2 px a report: 2000 px/s.
	let step = 2000.0 / (o.hz * f64::from(o.moves.max(1)));
	let count = (o.seconds * o.hz) as usize;
	let mut frames = Vec::with_capacity(count);
	let mut last = Instant::now();
	let mut next = last;
	let mut turns_ended = 0;
	let mut ended_at = last;
	for _ in 0..count {
		if o.end_turns.is_some_and(|every| ended_at.elapsed().as_secs_f64() >= every) {
			ended_at = Instant::now();
			if run.holds(BROTHERS_TURN) {
				run.game.run_script("::Tactical.TurnSequenceBar.onNextTurnButtonPressed();")?;
				turns_ended += 1;
			}
		}
		let start = Instant::now();
		let ms = start.duration_since(last).as_secs_f64() * 1000.0;
		last = start;
		run.frame(ms)?;
		let work = start.elapsed().as_secs_f64() * 1000.0;
		let spans: Vec<(&'static str, f64)> =
			frame_timing::take().into_iter().map(|(n, t)| (n, t.as_secs_f64() * 1000.0)).collect();
		let counts = frame_timing::take_counts();
		frames.push(Frame { interval: ms, work, spans, counts });
		// The mouse's moves, spread over the wait for the next frame; their handling, if
		// any, counts in the next frame's spans.
		next += interval;
		for i in 0..o.moves {
			let at = start + interval.mul_f64(f64::from(i + 1) / f64::from(o.moves + 1));
			if let Some(wait) = at.checked_duration_since(Instant::now()) {
				std::thread::sleep(wait);
			}
			let (x, y) = sweep.next(step);
			run.game.mouse_moved(x, y)?;
		}
		match next.checked_duration_since(Instant::now()) {
			Some(wait) => std::thread::sleep(wait),
			None => next = Instant::now(),
		}
	}
	let after = renderer_memory();
	let rested = run.game.front().page_rested() - rested;
	println!("the page rested through {rested} frames");
	frames.remove(0);
	report(o, &frames, before, after, turns_ended)?;
	Ok(())
}

fn report(o: &Options, frames: &[Frame], before: (f64, f64), after: (f64, f64), turns_ended: u32) -> Result<(), String> {
	let budget = 1000.0 / o.hz;
	let n = frames.len();
	let intervals: Vec<f64> = frames.iter().map(|f| f.interval).collect();
	let secs: f64 = intervals.iter().sum::<f64>() / 1000.0;
	let over = |t: f64| frames.iter().filter(|f| f.interval > t).count();
	// The lows: the frame rate the worst 1% (0.1%) of the time's frames ran at.
	let low = |share: f64| {
		let mut v: Vec<f64> = intervals.clone();
		v.sort_by(|a, b| b.partial_cmp(a).unwrap());
		let k = ((n as f64 * share).ceil() as usize).max(1);
		1000.0 / (v[..k].iter().sum::<f64>() / k as f64)
	};
	println!("battle hover: {} mods, {n} frames over {secs:.1} s at {} Hz, {} moves a frame; V8 flags {:?}",
		if o.mods { "with" } else { "no" }, o.hz, o.moves, std::env::var("BB_UI_V8_FLAGS").ok());
	let (low1, low01) = (low(0.01), low(0.001));
	let mut sorted = intervals.clone();
	println!(
		"interval ms: p50 {:.2}, p99 {:.2}, p99.9 {:.2}, max {:.2}; 1% low {low1:.0} fps, 0.1% low {low01:.0} fps",
		pct(&mut sorted, 0.5), pct(&mut sorted, 0.99), pct(&mut sorted, 0.999), pct(&mut sorted, 1.0)
	);
	println!(
		"intervals over 2x budget ({:.1} ms): {} ({:.1}/min); over 22 ms: {}; over 50 ms: {}",
		2.0 * budget, over(2.0 * budget), over(2.0 * budget) as f64 / secs * 60.0, over(22.0), over(50.0)
	);
	let mut names: Vec<&'static str> = Vec::new();
	for f in frames {
		for (k, _) in &f.spans {
			if !names.contains(k) {
				names.push(k);
			}
		}
	}
	println!("| span (ms a frame) | mean | p99 | p99.9 | max | total s |");
	println!("|---|---:|---:|---:|---:|---:|");
	let mut work: Vec<f64> = frames.iter().map(|f| f.work).collect();
	println!("| work (step+draw) | {:.3} | {:.2} | {:.2} | {:.2} | {:.2} |", work.iter().sum::<f64>() / n as f64,
		pct(&mut work, 0.99), pct(&mut work, 0.999), pct(&mut work, 1.0), work.iter().sum::<f64>() / 1000.0);
	for k in names {
		let mut v: Vec<f64> = frames.iter().map(|f| f.span(k)).collect();
		let total: f64 = v.iter().sum();
		if total / (n as f64) < 0.005 {
			continue;
		}
		println!("| {k} | {:.3} | {:.2} | {:.2} | {:.2} | {:.2} |", total / n as f64, pct(&mut v, 0.99), pct(&mut v, 0.999), pct(&mut v, 1.0), total / 1000.0);
	}
	let mut counters: Vec<(&'static str, u64)> = Vec::new();
	for f in frames {
		for (k, c) in &f.counts {
			match counters.iter_mut().find(|(n, _)| n == k) {
				Some((_, t)) => *t += c,
				None => counters.push((k, *c)),
			}
		}
	}
	let counters: Vec<String> = counters.iter().map(|(k, c)| format!("{k} {:.2}/frame", *c as f64 / n as f64)).collect();
	println!("counters: {}", counters.join(", "));
	// The waits for the page's own tasks: the step's park and the input's.
	let mut waits: Vec<f64> = frames.iter().map(|f| f.span("page park") + f.span("input park")).collect();
	let long = |t: f64| waits.iter().filter(|&&w| w > t).count();
	let (over5, over10) = (long(5.0), long(10.0));
	println!(
		"waits for the page's tasks (page park + input park) a frame: p99 {:.2}, p99.9 {:.2}, max {:.2} ms; over 5 ms {over5}, over 10 ms {over10}; brothers' turns ended {turns_ended}",
		pct(&mut waits, 0.99), pct(&mut waits, 0.999), pct(&mut waits, 1.0)
	);
	println!(
		"renderer memory: PSS {:.1} MB (V8 {:.1}) before the sweep, {:.1} MB (V8 {:.1}) after",
		before.0, before.1, after.0, after.1
	);
	if let Some(path) = &o.frames {
		let mut out = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?);
		for (i, f) in frames.iter().enumerate() {
			let mut line = format!("{i} interval={:.3} work={:.3}", f.interval, f.work);
			for (k, t) in &f.spans {
				line.push_str(&format!(" {}={t:.3}", k.replace(' ', "_")));
			}
			for (k, c) in &f.counts {
				line.push_str(&format!(" {}#={c}", k.replace(' ', "_")));
			}
			writeln!(out, "{line}").map_err(|e| e.to_string())?;
		}
	}
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let result = options().and_then(|o| run(&o));
	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("battle_hover: {e}");
			ExitCode::FAILURE
		}
	}
}
