//! Where the scripts' per-frame time goes, by function, file and mod: the game headless
//! with the install's mods (or none), a copy of a save loaded, then frames stepped on the
//! paused world map, in a town and with the world running at 1x, each profiled
//! ([`squirrel::ScriptProfiler`]).
//!
//! ```text
//! cargo run --release -p bb-engine --example script_profile -- \
//!     [--vanilla] [--only PREFIXES] [--save NAME | --new-campaign] [--saves DIR]
//!     [--frames N] [--hz HZ] [--phases paused,town,running,battle] [--top N] [--out REPORT.md]
//!     [--script FILE.nut] [--live-clock] [--frame-times FILE]
//! ```
//!
//! - `--script` runs a script once the world is up (e.g. to log what a phase does).
//! - `--frame-times` writes each unprofiled frame's time (ms), a line `phase ms` each.
//! - `--live-clock` puts a live clock behind `Time.getExactTime`, as `bb` and the
//!   original have it (the AI's background work then spreads over frames, and a script
//!   can time itself); the frame's time otherwise.
//! - `--phases` takes any of the three, `fast` (running at 4x), `battle` (the nearest hostile party engaged,
//!   both sides fought by the AI; best last, as the world isn't entered again), or
//!   `none`.
//!
//! - `--save NAME` copies `NAME.sav` from `--saves` (default: bb's savegames folder) into
//!   a scratch folder as `bb_probe_NAME.sav` and loads that; the original is only read.
//!   `--new-campaign` starts a new campaign instead (no save needed).
//! - Each phase steps 120 frames to settle, `--frames` (default 600) unprofiled to time
//!   them, then `--frames` profiled. A frame is the engine's frame and the headless UI
//!   client's replies (what `bb` steps as the scripts' part of a frame), at `--hz`
//!   (default 60) of real time.
//! - The report (Markdown, `--out` or stdout): per phase, the frame's time with and
//!   without profiling, the time by mod (each script's source traced to the archive it
//!   came from), the top functions by their own time and by inclusive time, and the call
//!   tree down to the paths that cost something.

use std::path::{Path, PathBuf};
use std::time::Instant;

use bb_engine::harness::{self, GameEngine};
use bb_engine::script_profile::{self, Owners};
use bb_engine::tactical_player as player;
use squirrel::ScriptProfiler;

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
const IN_WORLD: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";

struct Options {
	vanilla: bool,
	only: Option<Vec<String>>,
	save: Option<String>,
	saves: PathBuf,
	frames: u32,
	hz: f64,
	phases: Vec<String>,
	top: usize,
	out: Option<PathBuf>,
	script: Option<PathBuf>,
	live_clock: bool,
	frame_times: Option<PathBuf>,
}

fn options() -> Options {
	let mut o = Options {
		vanilla: false,
		only: None,
		save: None,
		saves: squirrel_bb::platform::saves_dir().unwrap_or_default(),
		frames: 600,
		hz: 60.0,
		phases: vec!["paused".into(), "town".into(), "running".into()],
		top: 30,
		out: None,
		script: None,
		live_clock: false,
		frame_times: None,
	};
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().unwrap_or_else(|| panic!("{arg} needs a value"));
		match arg.as_str() {
			"--vanilla" => o.vanilla = true,
			"--only" => o.only = Some(value().split(',').map(str::to_string).collect()),
			"--save" => o.save = Some(value()),
			"--new-campaign" => o.save = None,
			"--saves" => o.saves = PathBuf::from(value()),
			"--frames" => o.frames = value().parse().expect("--frames N"),
			"--hz" => o.hz = value().parse().expect("--hz HZ"),
			"--phases" => o.phases = value().split(',').filter(|p| *p != "none").map(str::to_string).collect(),
			"--top" => o.top = value().parse().expect("--top N"),
			"--out" => o.out = Some(PathBuf::from(value())),
			"--script" => o.script = Some(PathBuf::from(value())),
			"--live-clock" => o.live_clock = true,
			"--frame-times" => o.frame_times = Some(PathBuf::from(value())),
			other => panic!("unknown argument {other}"),
		}
	}
	o
}

fn main() {
	let o = options();
	let dir = PathBuf::from(harness::STEAM_DATA_DIR);
	let mut archives = harness::data_archives(&dir).expect("list the data archives");
	if !o.vanilla {
		let mut mods = harness::mod_archives(&dir).expect("list mods");
		if let Some(only) = &o.only {
			mods.retain(|m| {
				let name = m.file_name().unwrap().to_string_lossy();
				only.iter().any(|p| name.starts_with(p.as_str()))
			});
		}
		archives.extend(mods);
	}
	let owners = Owners::of(&archives);
	let live = o.live_clock;
	let mut engine = harness::open_mounted(&dir, archives, |c| {
		if live {
			c.exact_clock = squirrel_bb::ExactClock::Live;
		}
	})
	.expect("open the install");
	engine.vm_mut().install_standard_error_handlers();

	// A scratch folder of our own for the saves (the game writes some, e.g. MSU's).
	let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/tmp/script_profile");
	let _ = std::fs::remove_dir_all(&scratch);
	std::fs::create_dir_all(&scratch).expect("make the scratch folder");
	engine.saves().set_directory(scratch.clone());
	let load = o.save.as_ref().map(|name| {
		let copy = format!("bb_probe_{name}");
		std::fs::copy(o.saves.join(format!("{name}.sav")), scratch.join(format!("{copy}.sav"))).expect("copy the save");
		copy
	});

	let t = Instant::now();
	harness::boot_menu(&engine).expect("boot the menu");
	until(&engine, 60, "main menu", &format!("{MENU}.isVisible() && !{MENU}.isAnimating()"));
	eprintln!("menu in {:.1} s", t.elapsed().as_secs_f64());
	match &load {
		Some(name) => load_campaign(&engine, name),
		None => new_campaign(&engine),
	}
	eprintln!("in the world after {:.1} s", t.elapsed().as_secs_f64());
	if let Some(path) = &o.script {
		script(&engine, "--script", &std::fs::read_to_string(path).expect("read --script"));
	}

	let mut text = format!(
		"# Script profile: {}{}\n\n{} frames per phase at {} Hz real time.\n\n",
		if o.vanilla { "vanilla" } else { "the install's mods" },
		load.as_ref().map_or(", a new campaign".to_string(), |s| format!(", save {s}")),
		o.frames,
		o.hz
	);
	let mut frame_log = String::new();
	for phase in &o.phases {
		enter_phase(&engine, phase, load.is_some());
		let dt = 1000.0 / o.hz;
		for _ in 0..120 {
			step(&engine, dt, None);
		}
		let (plain, spread, times) = timed(&engine, dt, o.frames, None);
		frame_log += &times.iter().map(|t| format!("{phase} {t:.4}\n")).collect::<String>();
		let mut profiler = ScriptProfiler::new();
		let (profiled, _, _) = timed(&engine, dt, o.frames, Some(&mut profiler));
		let state = player::eval_string(&engine, STATE).ok().flatten().unwrap_or_default();
		let notes = format!(
			"{state}\n\n- a frame (the engine's frame and the UI client's replies): {plain:.3} ms unprofiled (p50 {:.3}, p99 {:.3}, max {:.3}), {profiled:.3} ms profiled",
			spread.0, spread.1, spread.2
		);
		text += &script_profile::report(phase, &notes, &profiler.profile(), &owners, o.top);
	}
	match &o.out {
		Some(path) => std::fs::write(path, &text).expect("write the report"),
		None => print!("{text}"),
	}
	if let Some(path) = &o.frame_times {
		std::fs::write(path, &frame_log).expect("write the frame times");
	}
	cleanup(&scratch);
}

/// What the world looks like when a phase is profiled.
const STATE: &str = "\"paused \" + ::World.State.isPaused() + \", time speed \" + ::World.getSpeedMult() + \", town \" + (::World.State.m.LastEnteredTown != null ? ::World.State.m.LastEnteredTown.getName() : \"none\") + \", day \" + ::World.getTime().Days";

fn enter_phase(engine: &GameEngine, phase: &str, loaded: bool) {
	let leave_town = "if (::World.State.m.LastEnteredTown != null) ::World.State.m.MenuStack.pop(true);";
	if phase == "running" || phase == "fast" {
		// Leaving a town pauses again a few frames on (AutoPauseAfterCity).
		script(engine, "leave the town", leave_town);
		for _ in 0..30 {
			step(engine, 1000.0 / 60.0, None);
		}
	}
	let src = match phase {
		"paused" => format!("{leave_town} ::World.State.setPause(true);"),
		"town" => "local p = ::World.State.getPlayer(); local best = null; local d = 0;
			foreach (s in ::World.EntityManager.getSettlements())
				if (best == null || s.getTile().getDistanceTo(p.getTile()) < d) { best = s; d = s.getTile().getDistanceTo(p.getTile()); }
			::World.State.enterLocation(best);"
			.to_string(),
		// Hostile parties about to attack the company leave first (a battle would start).
		"running" => format!(
			"local p = ::World.State.getPlayer();
			foreach (e in ::World.getAllEntitiesAtPos(p.getPos(), 600.0))
				if (e.isParty() && e.getID() != p.getID() && !e.isAlliedWithPlayer()) e.die();
			::World.State.setAutoPause(false); ::World.State.setPause(false); ::World.State.setNormalTime();"
		),
		"battle" => {
			enter_battle(engine, loaded);
			return;
		}
		// As `running`, at the fast speed (4x).
		"fast" => format!(
			"local p = ::World.State.getPlayer();
			foreach (e in ::World.getAllEntitiesAtPos(p.getPos(), 600.0))
				if (e.isParty() && e.getID() != p.getID() && !e.isAlliedWithPlayer()) e.die();
			::World.State.setAutoPause(false); ::World.State.setPause(false); ::World.State.setNormalTime(); ::World.setSpeedMult(4.0);"
		),
		other => panic!("unknown phase {other} (paused, town, running, fast, battle)"),
	};
	script(engine, phase, &src);
}

/// Into a battle with the nearest hostile party, both sides fought by the AI (see
/// `harness::install_autoplay`), once its first round is under way.
fn enter_battle(engine: &GameEngine, loaded: bool) {
	script(engine, "leave the town", "if (::World.State.m.LastEnteredTown != null) ::World.State.m.MenuStack.pop(true);");
	harness::install_autoplay(engine).expect("install the autoplay");
	script(engine, "fight", "::__autoplayFight = true; ::__autoplayDieAsPlayers = true; ::World.State.setPause(false);");
	// A new campaign first runs past its origin's intro, which would stop the battle.
	if loaded { harness::offer_nearest_battle(engine) } else { harness::engage_nearest_hostile(engine) }.expect("engage");
	let started = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible() && ::Time.getRound() >= 1";
	for _ in 0..3000 {
		if player::eval_i64(engine, &format!("({started}) ? 1 : 0")).ok().flatten() == Some(1) {
			return;
		}
		step(engine, 1000.0 / 60.0, None);
	}
	panic!("battle: not started after 3000 frames");
}

/// Step `frames` frames, profiled into `profiler` if given: the mean frame in ms, the
/// frames' p50, p99 and max, and every frame's time in order.
fn timed(engine: &GameEngine, dt: f64, frames: u32, mut profiler: Option<&mut ScriptProfiler>) -> (f64, (f64, f64, f64), Vec<f64>) {
	let times: Vec<f64> = (0..frames).map(|_| step(engine, dt, profiler.as_deref_mut())).collect();
	let mean = times.iter().sum::<f64>() / frames.max(1) as f64;
	let mut sorted = times.clone();
	sorted.sort_by(f64::total_cmp);
	let at = |q: f64| sorted.get(((sorted.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
	(mean, (at(0.5), at(0.99), at(1.0)), times)
}

/// One frame: the engine's frame and the UI client's replies (profiled into `profiler`
/// if given), then the harness's scripted player (never profiled). The ms the first part
/// took.
fn step(engine: &GameEngine, dt: f64, profiler: Option<&mut ScriptProfiler>) -> f64 {
	let run = || -> Result<(), String> {
		engine.frame(dt)?;
		engine.pump_ui().map(|_| ())
	};
	let t = Instant::now();
	let result = match profiler {
		Some(p) => engine.vm().profiled(p, run),
		None => run(),
	};
	let ms = t.elapsed().as_secs_f64() * 1000.0;
	if let Err(e) = result {
		eprintln!("frame: {e}");
	}
	let autoplay = squirrel::SqTable::root(engine.vm()).get::<squirrel::SqStrongRef>("__autoplay");
	if let Some(f) = autoplay {
		let _ = f.call(engine.vm(), ());
	}
	ms
}

fn script(engine: &GameEngine, what: &str, src: &str) {
	if let Err(e) = engine.run_script(src, what) {
		panic!("{what}: {e}");
	}
}

fn until(engine: &GameEngine, frames: u32, what: &str, condition: &str) {
	for _ in 0..frames {
		if player::eval_i64(engine, &format!("({condition}) ? 1 : 0")).ok().flatten() == Some(1) {
			return;
		}
		harness::run_frame(engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("{what}: {e}"));
	}
	panic!("{what}: not after {frames} frames");
}

/// Load `name` from the main menu, as a player does.
fn load_campaign(engine: &GameEngine, name: &str) {
	// The world state ignores a load within 4 s (real time) of the game's start.
	until(engine, 600, "4 s at the menu", "::Time.getRealTimeF() > 4.5");
	script(engine, "load menu", &format!("{MENU}.getMainMenuModule().onLoadCampaignButtonPressed();"));
	harness::run_frame(engine, 1000.0 / 60.0).expect("frame");
	script(engine, "load", &format!("{MENU}.getLoadCampaignMenuModule().onLoadButtonPressed(\"{name}\");"));
	until(engine, 1200, "the campaign loads", IN_WORLD);
}

fn new_campaign(engine: &GameEngine) {
	harness::prepare_new_campaign(engine, "script-profile").expect("prepare the campaign");
	harness::start_new_campaign(engine).expect("start the campaign");
	// MSU's new-campaign settings page, as a player saves it.
	script(engine, "MSU settings", "if (\"MSU\" in getroottable()) ::MSU.SettingsScreen.onSaveButtonPressed({});");
	harness::run_past_intro(engine).expect("past the intro");
}

/// Remove the scratch folder's saves (all ours: `bb_probe_*` and what the mods wrote).
fn cleanup(scratch: &Path) {
	let _ = std::fs::remove_dir_all(scratch);
}
