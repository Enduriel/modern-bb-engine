//! A session: one game process, playing campaigns until the run's time is up. This file
//! has its lifecycle (boot, the main loop, heartbeats, stalls); `world.rs`, `battle.rs`
//! and `saveload.rs` have what it does in each part of the game.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bb_engine::harness::{self, GameEngine};
use bb_engine::tactical_player as player;

use crate::Options;
use crate::handlers::{self, Shared};
use crate::record::{self, Json, clip, signature, tell};
use crate::supervisor::{EXIT_BOOT, EXIT_DONE, EXIT_PANIC, EXIT_STALLED};

/// A frame's real time: the game runs as fast as it can, each frame 1/60 s on its clocks.
pub const FRAME_MS: f64 = 1000.0 / 60.0;
/// Frames without any change before the game counts as stalled.
const STALL_FRAMES: i64 = 1800;
/// A frame error repeated this many frames in a row ends the session.
const FRAME_ERRORS_IN_A_ROW: u32 = 600;
/// Frames of progress after a stall before the stall ladder starts again from its
/// first step (four times a stall's length).
const STALL_RESET_FRAMES: i64 = 4 * STALL_FRAMES;

/// Where the game is, as the main loop sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
	Menu,
	Loading,
	World,
	Town,
	Tactical,
	GameOver,
}

const PHASE: &str = r#"(function() {
	if (::__soakInTactical()) return "tactical";
	if (::RootState.get("WorldState") == null) return "menu";
	if (!("State" in ::World) || ::World.State == null || ::World.State.getPlayer() == null || ::LoadingScreen.isVisible()) return "loading";
	if (::World.State.m.IsGameOver) return "gameover";
	if (::World.State.m.WorldTownScreen.isVisible() && ::World.State.getCurrentTown() != null) return "town";
	return "world";
})()"#;

/// How a session ends.
pub enum End {
	Deadline,
	Stalled(String),
}

/// Counters for the run's summary.
#[derive(Default)]
pub struct Stats {
	pub campaigns: i64,
	pub games_over: i64,
	pub days: i64,
	pub battles: i64,
	pub battles_won: i64,
	pub battles_weakened: i64,
	/// Battles the company fought with at most one brother on the field (of more).
	pub battles_lone: i64,
	pub events_answered: i64,
	pub contracts_offered: i64,
	pub contracts_taken: i64,
	pub contracts_ended: i64,
	pub towns_entered: i64,
	pub sold: i64,
	pub bought: i64,
	pub hired: i64,
	pub saves: i64,
	pub loads: i64,
	pub save_failures: i64,
	pub load_failures: i64,
	pub invariant_mismatches: i64,
	pub keep_alive: i64,
	pub stalls: i64,
	pub frames: i64,
}

impl Stats {
	fn line(&self) -> String {
		format!(
			"campaigns={} games_over={} days={} battles={} battles_won={} battles_weakened={} battles_lone={} events_answered={} contracts_offered={} contracts_taken={} contracts_ended={} towns_entered={} sold={} bought={} hired={} saves={} loads={} save_failures={} load_failures={} invariant_mismatches={} keep_alive={} stalls={} frames={}",
			self.campaigns,
			self.games_over,
			self.days,
			self.battles,
			self.battles_won,
			self.battles_weakened,
			self.battles_lone,
			self.events_answered,
			self.contracts_offered,
			self.contracts_taken,
			self.contracts_ended,
			self.towns_entered,
			self.sold,
			self.bought,
			self.hired,
			self.saves,
			self.loads,
			self.save_failures,
			self.load_failures,
			self.invariant_mismatches,
			self.keep_alive,
			self.stalls,
			self.frames
		)
	}
}

pub struct Session<'a> {
	pub o: &'a Options,
	pub n: u32,
	pub engine: GameEngine,
	pub shared: Rc<Shared>,
	pub stats: Stats,
	pub save_dir: PathBuf,
	actions: Option<File>,
	started: Instant,
	deadline: u64,
	rng: u64,
	/// The campaign's seed, and how many campaigns this session has started.
	pub campaign_seed: String,
	/// The day the current campaign's days are counted from, and the last day seen.
	pub day_from: i64,
	pub day_now: i64,
	pub next_save_day: i64,
	pub save_count: u32,
	frame_errors_in_a_row: u32,
	/// The last progress fingerprint, how many ticks it hasn't changed, and the stall
	/// level reached (each level tries harder to get out; the last gives up).
	fingerprint: String,
	still_ticks: u32,
	still_since: i64,
	pub stall_level: u32,
	/// The frame of the last stall: the ladder steps back down only after
	/// [`STALL_RESET_FRAMES`] of progress since (a nudge's own stir is not a way out).
	last_stall: i64,
	last_beat: Instant,
	last_stat: Instant,
	last_stubs: Instant,
	stubs: BTreeMap<String, u64>,
	pub phase: Phase,
	pub world: crate::world::WorldPlay,
	pub battle: crate::battle::BattlePlay,
	injected: Option<String>,
	/// `--inject stall`: frames no longer run.
	frozen: bool,
}

pub fn main(o: &Options, n: u32) -> ExitCode {
	let events = o.out.join("events.jsonl");
	if let Err(e) = record::open(&events, n, &o.seed) {
		eprintln!("{}: {e}", events.display());
		return ExitCode::from(EXIT_BOOT as u8);
	}
	record::install_panic_hook();
	allow_stack_dumps();
	let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| run(o, n)));
	let code = match outcome {
		Ok(Ok(End::Deadline)) => EXIT_DONE,
		Ok(Ok(End::Stalled(why))) => {
			eprintln!("soak: gave up: {why}");
			EXIT_STALLED
		}
		Ok(Err(e)) => {
			eprintln!("soak: {e}");
			Json::new("boot_failure").str("message", &e).emit();
			tell("SIG", &format!("boot_failure\t{}", signature(&e)));
			EXIT_BOOT
		}
		// The panic hook recorded it. Leave without dropping anything more.
		Err(_) => EXIT_PANIC,
	};
	let _ = std::io::stdout().flush();
	std::process::exit(code);
}

/// Let the supervisor's `eu-stack` attach (Yama's ptrace scope 1 lets only ancestors).
/// Linux only: Yama and `prctl` are Linux's.
#[cfg(target_os = "linux")]
fn allow_stack_dumps() {
	unsafe extern "C" {
		fn prctl(option: std::ffi::c_int, ...) -> std::ffi::c_int;
	}
	const PR_SET_PTRACER: std::ffi::c_int = 0x5961_6d61;
	const PR_SET_PTRACER_ANY: std::ffi::c_ulong = std::ffi::c_ulong::MAX;
	// SAFETY: prctl(PR_SET_PTRACER, any) only changes who may trace this process.
	unsafe {
		prctl(PR_SET_PTRACER, PR_SET_PTRACER_ANY);
	}
}

#[cfg(not(target_os = "linux"))]
fn allow_stack_dumps() {}

fn run(o: &Options, n: u32) -> Result<End, String> {
	let io = |e: std::io::Error| e.to_string();
	let mut archives = harness::data_archives(&o.data).map_err(io)?;
	let mods = if o.mods { harness::mod_archives(&o.data).map_err(io)? } else { Vec::new() };
	archives.extend(mods.iter().cloned());
	let mut engine = harness::open_mounted(&o.data, archives, |_| {}).map_err(io)?;
	// Saves go to a folder of the workspace's target/, never the game's own save folder.
	let tag = o.out.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "run".into());
	let save_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("../target/soak-saves")
		.join(format!("{tag}-{}-{n}", std::process::id()));
	std::fs::create_dir_all(&save_dir).map_err(|e| format!("{}: {e}", save_dir.display()))?;
	engine.saves().set_directory(save_dir.clone());
	engine.saves().set_modified(!mods.is_empty());
	let shared = Shared::new(File::create(o.out.join(format!("game-{n}.log"))).ok());
	handlers::install(&mut engine, &shared);
	let deadline = o.deadline.unwrap_or(u64::MAX);
	let seed_hash = o.seed.bytes().fold(5381u64, |h, b| h.wrapping_mul(33) ^ b as u64);
	let mut s = Session {
		o,
		n,
		engine,
		shared,
		stats: Stats::default(),
		save_dir,
		actions: File::create(o.out.join(format!("actions-{n}.log"))).ok(),
		started: Instant::now(),
		deadline,
		rng: seed_hash | 1,
		campaign_seed: o.seed.clone(),
		day_from: 0,
		day_now: 0,
		next_save_day: 0,
		save_count: 0,
		frame_errors_in_a_row: 0,
		fingerprint: String::new(),
		still_ticks: 0,
		still_since: 0,
		stall_level: 0,
		last_stall: 0,
		last_beat: Instant::now(),
		last_stat: Instant::now(),
		last_stubs: Instant::now(),
		stubs: BTreeMap::new(),
		phase: Phase::Menu,
		world: Default::default(),
		battle: Default::default(),
		injected: o.inject.clone(),
		frozen: false,
	};
	s.action(&format!("session {n}: {} mods, scenario {}, seed {}", mods.len(), o.scenario, o.seed));
	s.context("boot");
	harness::boot_menu(&s.engine)?;
	harness::install_autoplay(&s.engine)?;
	let soak = include_str!("soak.nut").replace("@SEED@", &((seed_hash & 0x7fff_ffff) as i64).to_string());
	s.engine.run_script(&soak, "soak.nut")?;
	s.start_campaign()?;
	let end = s.play();
	s.finish(&end);
	Ok(end)
}

impl Session<'_> {
	/// The session's own random numbers, for its choices (a 64-bit xorshift).
	pub fn rand(&mut self, n: u64) -> u64 {
		self.rng ^= self.rng << 13;
		self.rng ^= self.rng >> 7;
		self.rng ^= self.rng << 17;
		if n == 0 { 0 } else { self.rng % n }
	}

	/// Note what the session is doing (for its errors' context).
	pub fn context(&self, what: &str) {
		*self.shared.context.borrow_mut() = format!("{what} (day {})", self.day_now);
	}

	/// A line in the session's action log.
	pub fn action(&mut self, what: &str) {
		let secs = self.started.elapsed().as_secs_f64();
		let day = self.day_now;
		if let Some(f) = self.actions.as_mut() {
			let _ = writeln!(f, "[{secs:8.1}s day {day:4}] {what}");
		}
	}

	/// Run `code`; an error is recorded (the script error itself, with its stack, was
	/// recorded by the handler) and returned.
	pub fn script(&self, what: &str, code: &str) -> Result<(), String> {
		self.engine.run_script(code, what).inspect_err(|e| self.call_failed(what, e))
	}

	/// A string expression's value ("" if it failed, which is recorded).
	pub fn eval(&self, what: &str, expr: &str) -> String {
		match player::eval_string(&self.engine, expr) {
			Ok(v) => v.unwrap_or_default(),
			Err(e) => {
				self.call_failed(what, &e);
				String::new()
			}
		}
	}

	/// An integer expression's value (None if it failed, which is recorded).
	pub fn eval_i64(&self, what: &str, expr: &str) -> Option<i64> {
		player::eval_i64(&self.engine, expr).inspect_err(|e| self.call_failed(what, e)).ok().flatten()
	}

	pub fn holds(&self, what: &str, cond: &str) -> bool {
		self.eval_i64(what, &format!("({cond}) ? 1 : 0")) == Some(1)
	}

	fn call_failed(&self, what: &str, e: &str) {
		let sig = signature(&format!("{what}: {e}"));
		self.shared.error("soak_call_error", &sig, Json::new("soak_call_error").str("call", what).str("message", &clip(e, 4000)));
	}

	/// Run `count` frames; frame errors are recorded. Ends the session (an `Err`) when the
	/// same frame error repeats for [`FRAME_ERRORS_IN_A_ROW`] frames.
	pub fn frames(&mut self, count: u32) -> Result<(), End> {
		for _ in 0..count {
			if self.frozen {
				break;
			}
			match harness::run_frame(&self.engine, FRAME_MS) {
				Ok(()) => self.frame_errors_in_a_row = 0,
				Err(e) => {
					self.frame_errors_in_a_row += 1;
					let sig = signature(&e);
					self.shared.error("frame_error", &sig, Json::new("frame_error").str("message", &clip(&e, 4000)));
					if self.frame_errors_in_a_row >= FRAME_ERRORS_IN_A_ROW {
						return Err(End::Stalled(format!("{FRAME_ERRORS_IN_A_ROW} frames in a row failed: {}", clip(&e, 300))));
					}
				}
			}
			self.stats.frames += 1;
		}
		self.beat();
		Ok(())
	}

	/// Run frames until `cond` holds (true) or `max` frames have run (false).
	pub fn until(&mut self, max: u32, cond: &str) -> Result<bool, End> {
		for _ in 0..max / 10 {
			if self.holds("wait", cond) {
				return Ok(true);
			}
			self.frames(10)?;
		}
		Ok(self.holds("wait", cond))
	}

	/// The heartbeat (every second), counters (every 5 s) and new stubs (every minute)
	/// for the supervisor.
	fn beat(&mut self) {
		if self.last_beat.elapsed() >= Duration::from_secs(1) {
			self.last_beat = Instant::now();
			let state = self.eval("heartbeat", "::__soakState(false)");
			tell("HB", &format!("{:?} {state}", self.phase));
		}
		if self.last_stat.elapsed() >= Duration::from_secs(5) {
			self.last_stat = Instant::now();
			tell("STAT", &self.stats.line());
		}
		if self.last_stubs.elapsed() >= Duration::from_secs(60) {
			self.last_stubs = Instant::now();
			self.check_stubs();
		}
	}

	/// Record each stub the scripts reached for the first time.
	fn check_stubs(&mut self) {
		let calls = self.engine.stub_calls();
		let fresh: Vec<(String, u64)> = calls.iter().filter(|(name, _)| !self.stubs.contains_key(name)).cloned().collect();
		if !fresh.is_empty() {
			let state = self.eval("stub state", "::__soakState(false)");
			for (name, count) in &fresh {
				tell("SIG", &format!("stub\t{name}"));
				Json::new("stub").str("name", name).int("calls", *count as i64).str("state", &state).emit();
			}
		}
		self.stubs = calls.into_iter().collect();
	}

	/// The `--inject` fault, once, 20 s into the session.
	fn maybe_inject(&mut self) {
		if self.started.elapsed() < Duration::from_secs(20) || self.phase != Phase::World {
			return;
		}
		let Some(what) = self.injected.take() else {
			return;
		};
		self.action(&format!("INJECT {what}"));
		match what.as_str() {
			"hang" => {
				let _ = self.engine.run_script("while (true) {}", "inject");
			}
			"panic" => panic!("injected panic"),
			"abort" => std::process::abort(),
			// The game no longer runs: what a game stuck for good looks like to the loop.
			"stall" => self.frozen = true,
			"gameover" => {
				let _ = self.script("inject", "foreach (b in ::World.getPlayerRoster().getAll()) ::World.getPlayerRoster().remove(b);");
			}
			other => self.action(&format!("unknown injection {other}")),
		}
	}

	fn past_deadline(&self) -> bool {
		SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) >= self.deadline
	}

	pub fn phase_now(&self) -> Phase {
		match self.eval("phase", PHASE).as_str() {
			"tactical" => Phase::Tactical,
			"menu" => Phase::Menu,
			"world" => Phase::World,
			"town" => Phase::Town,
			"gameover" => Phase::GameOver,
			_ => Phase::Loading,
		}
	}

	/// The main loop: a tick at a time (some frames, then the player's next move), until
	/// the run's time is up or a stall can't be got out of.
	fn play(&mut self) -> End {
		loop {
			if self.past_deadline() {
				return End::Deadline;
			}
			self.maybe_inject();
			let phase = self.phase_now();
			if phase != self.phase {
				self.action(&format!("phase {:?} -> {phase:?}", self.phase));
			}
			let was = self.phase;
			self.phase = phase;
			if phase != Phase::Tactical && was == Phase::Tactical {
				self.battle_over();
			}
			let step = match phase {
				Phase::Tactical => self.battle_tick(was != Phase::Tactical),
				Phase::World => self.world_tick(was),
				Phase::Town => self.town_tick(was != Phase::Town),
				Phase::GameOver => self.game_over(),
				Phase::Menu => self.at_menu(),
				Phase::Loading => self.frames(10),
			};
			if let Err(end) = step {
				return end;
			}
			if let Err(end) = self.watch_progress() {
				return end;
			}
		}
	}

	/// Whether the game moves on: its clocks, its screens, the battle. A tick without
	/// change is still; enough still ticks are a stall, recorded with what's up, and met
	/// with harder and harder nudges, the last of which gives up on the session.
	fn watch_progress(&mut self) -> Result<(), End> {
		let print = self.eval("progress", "::__soakState(false)");
		if print != self.fingerprint {
			self.fingerprint = print;
			self.still_ticks = 0;
			self.still_since = self.stats.frames;
			if self.stats.frames - self.last_stall >= STALL_RESET_FRAMES {
				self.stall_level = 0;
			}
			return Ok(());
		}
		self.still_ticks += 1;
		// Half a minute of the game's time still (or ticks that no longer run frames).
		if self.stats.frames - self.still_since < STALL_FRAMES && self.still_ticks < 600 {
			return Ok(());
		}
		self.still_ticks = 0;
		self.still_since = self.stats.frames;
		self.last_stall = self.stats.frames;
		self.stall_level += 1;
		self.stats.stalls += 1;
		let detail = self.eval("stall state", "::__soakState(true)");
		let sig = signature(&format!("{:?}: {}", self.phase, stall_shape(&detail)));
		self.shared.error(
			"stall",
			&sig,
			Json::new("stall").str("phase", &format!("{:?}", self.phase)).int("level", self.stall_level as i64).str("state", &detail),
		);
		self.action(&format!("STALL level {}: {detail}", self.stall_level));
		match self.stall_level {
			1 => {
				let _ = self.script(
					"unstick 1",
					"if (::__soakInTactical()) {} else if (\"State\" in ::World && ::World.State != null) { ::World.State.setAutoPause(false); ::World.State.setPause(false); }",
				);
				if self.phase == Phase::Tactical {
					self.weaken();
				}
				Ok(())
			}
			2 => {
				let _ = self.script(
					"unstick 2",
					"if (!::__soakInTactical() && \"State\" in ::World && ::World.State != null) { ::World.State.m.MenuStack.popAll(true); ::World.State.setAutoPause(false); ::World.State.setPause(false); }",
				);
				if self.phase == Phase::Tactical {
					self.weaken();
				}
				Ok(())
			}
			_ => Err(End::Stalled(format!("{:?}: {detail}", self.phase))),
		}
	}

	/// The session's summary record, and the counters' last line.
	fn finish(&mut self, end: &End) {
		self.check_stubs();
		let why = match end {
			End::Deadline => "time up".to_string(),
			End::Stalled(why) => format!("gave up: {why}"),
		};
		self.action(&format!("end: {why}"));
		let errors: Vec<String> = self
			.shared
			.error_counts()
			.iter()
			.map(|((kind, sig), n)| format!("{{\"kind\":{},\"signature\":{},\"count\":{n}}}", record::quote(kind), record::quote(sig)))
			.collect();
		let warnings: Vec<String> = self
			.shared
			.warning_counts()
			.iter()
			.take(50)
			.map(|(sig, n)| format!("{{\"signature\":{},\"count\":{n}}}", record::quote(sig)))
			.collect();
		let stubs: Vec<String> = self.stubs.iter().map(|(k, v)| format!("{}:{v}", record::quote(k))).collect();
		let stats: Vec<String> = self
			.stats
			.line()
			.split_whitespace()
			.filter_map(|kv| kv.split_once('='))
			.map(|(k, v)| format!("{}:{v}", record::quote(k)))
			.collect();
		Json::new("session_summary")
			.str("end", &why)
			.num("minutes", self.started.elapsed().as_secs_f64() / 60.0)
			.raw("stats", &format!("{{{}}}", stats.join(",")))
			.raw("errors", &format!("[{}]", errors.join(",")))
			.raw("warnings", &format!("[{}]", warnings.join(",")))
			.raw("stubs", &format!("{{{}}}", stubs.join(",")))
			.emit();
		tell("STAT", &self.stats.line());
		// The last save stays for a look; the rest went as they were checked.
		let _ = std::io::stdout().flush();
	}
}

/// A stall's state with its numbers masked, for its signature (the day and the clock
/// differ from stall to stall).
fn stall_shape(detail: &str) -> String {
	detail
		.split(", ")
		.filter(|part| !part.starts_with("world day") && !part.starts_with("money") && !part.starts_with("battle round"))
		.collect::<Vec<_>>()
		.join(", ")
}
