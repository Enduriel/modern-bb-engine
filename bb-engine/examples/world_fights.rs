//! Run a campaign's world headless for many days and record its auto-resolved world-map
//! fights (the script combat manager, `World.Combat`): who fought whom, with what
//! factions and strengths, how each ended, fights that never end, parties left "in
//! combat" with no combat, and every script error. Made to diagnose world-size and
//! faction-count mods (the world_parameters reports): a mod's settings are set by a
//! script before the campaign starts.
//!
//! ```text
//! world_fights [--data <install>/data] [--mods mod_modern_hooks,mod_msu] [--extra a.zip,b.zip]
//!              [--settings settings.nut] [--world-script probe.nut] [--seed s] [--days 30] [--speed 8]
//!              [--save-every-days 0] [--chain 1] [--saves DIR] [--profile-start REPORT.md] --out run.log
//! ```
//!
//! - `--mods` mounts the install's mods whose file names start with one of the prefixes
//!   (none by default); `--extra` mounts more zips after them, from anywhere.
//! - `--settings` runs a script at the main menu, before New Campaign (set a mod's MSU
//!   settings there). `--world-script` runs a script once the campaign has started
//!   (after the fight recorder is installed); `PROBE` lines it logs go to the log.
//! - `--profile-start REPORT.md` profiles the campaign's start (worldgen and the faction
//!   simulation) by script function and native.
//! - The player's company is made unattackable, fed and paid, and the event manager is
//!   stopped, so the world runs on its own. The world speed is `--speed`.
//! - While a probe run holds the game, `data/` may also hold the zips it was
//!   given (`--with-mod`): a prefix in `--mods` matches those too. The log lists every
//!   zip mounted.
//! - `--save-every-days` saves (`bb_probe_wf_<k>`, in `--saves`, default
//!   `target/world-fights-saves`) and loads back in the world every so many days,
//!   `--chain` times in a row (a save of a just-loaded campaign, as a player's next
//!   autosave would be).
//! - `--out` gets the log: the fights (`WF` lines), the game's warnings and errors, the
//!   script errors with their stacks, worldgen and load timings.

use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use bb_engine::{harness, tactical_player};
use squirrel::ErrorValue;
use squirrel_bb::LogLevel;

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
const IN_WORLD: &str = "(\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible() && !::World.State.m.IsGameOver";
const FRAME_MS: f64 = 1000.0 / 60.0;

struct Options {
	data: PathBuf,
	mods: Vec<String>,
	extra: Vec<PathBuf>,
	settings: Option<PathBuf>,
	world_script: Option<PathBuf>,
	profile_start: Option<PathBuf>,
	seed: String,
	days: i64,
	speed: f64,
	save_every_days: i64,
	chain: u32,
	saves: PathBuf,
	out: PathBuf,
}

fn options() -> Result<Options, String> {
	let mut o = Options {
		data: PathBuf::from(harness::STEAM_DATA_DIR),
		mods: Vec::new(),
		extra: Vec::new(),
		settings: None,
		world_script: None,
		profile_start: None,
		seed: "world-fights".into(),
		days: 30,
		speed: 8.0,
		save_every_days: 0,
		chain: 1,
		saves: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/world-fights-saves"),
		out: PathBuf::new(),
	};
	let mut args = std::env::args().skip(1);
	while let Some(a) = args.next() {
		let mut v = || args.next().ok_or(format!("{a} needs a value"));
		match a.as_str() {
			"--data" => o.data = v()?.into(),
			"--mods" => o.mods = v()?.split(',').map(String::from).collect(),
			"--extra" => o.extra = v()?.split(',').map(PathBuf::from).collect(),
			"--settings" => o.settings = Some(v()?.into()),
			"--world-script" => o.world_script = Some(v()?.into()),
			"--profile-start" => o.profile_start = Some(v()?.into()),
			"--seed" => o.seed = v()?,
			"--days" => o.days = v()?.parse().map_err(|e| format!("--days: {e}"))?,
			"--speed" => o.speed = v()?.parse().map_err(|e| format!("--speed: {e}"))?,
			"--save-every-days" => {
				o.save_every_days = v()?
					.parse()
					.map_err(|e| format!("--save-every-days: {e}"))?
			}
			"--chain" => o.chain = v()?.parse().map_err(|e| format!("--chain: {e}"))?,
			"--saves" => o.saves = v()?.into(),
			"--out" => o.out = v()?.into(),
			_ => {
				return Err(format!(
					"unknown argument {a} (see the example's doc comment)"
				));
			}
		}
	}
	if o.out.as_os_str().is_empty() {
		return Err("--out <file> is required".into());
	}
	Ok(o)
}

type Log = Rc<RefCell<std::fs::File>>;

fn line(log: &Log, s: &str) {
	let _ = writeln!(log.borrow_mut(), "{s}");
}

fn main() -> std::process::ExitCode {
	let o = match options() {
		Ok(o) => o,
		Err(e) => {
			eprintln!("{e}");
			return std::process::ExitCode::FAILURE;
		}
	};
	let log: Log = Rc::new(RefCell::new(
		std::fs::File::create(&o.out).expect("create --out"),
	));
	match run(&o, &log) {
		Ok(()) => std::process::ExitCode::SUCCESS,
		Err(e) => {
			line(&log, &format!("STOPPED: {e}"));
			eprintln!("stopped: {e}");
			std::process::ExitCode::FAILURE
		}
	}
}

fn run(o: &Options, log: &Log) -> Result<(), String> {
	let io = |e: std::io::Error| e.to_string();
	let mut archives = harness::data_archives(&o.data).map_err(io)?;
	let mut mods = harness::mod_archives(&o.data).map_err(io)?;
	mods.retain(|m| {
		let name = m.file_name().unwrap().to_string_lossy();
		o.mods.iter().any(|p| name.starts_with(p.as_str()))
	});
	mods.extend(o.extra.iter().cloned());
	for m in &mods {
		line(log, &format!("mod {}", m.display()));
	}
	archives.extend(mods.iter().cloned());
	let mut engine = harness::open_mounted(&o.data, archives.clone(), |_| {}).map_err(io)?;
	std::fs::create_dir_all(&o.saves).map_err(io)?;
	engine.saves().set_directory(o.saves.clone());
	engine.saves().set_modified(!mods.is_empty());
	let errors = Rc::new(RefCell::new(0u64));
	{
		let (log, errors) = (log.clone(), errors.clone());
		engine.vm_mut().set_runtime_error_handler(move |error| {
			*errors.borrow_mut() += 1;
			let message = match &error.error {
				ErrorValue::String(b) => String::from_utf8_lossy(b).into_owned(),
				_ => "(a thrown non-string)".into(),
			};
			let stack: Vec<String> = error
				.frames
				.iter()
				.take(6)
				.map(|f| {
					format!(
						"{} ({}:{})",
						f.function.as_deref().unwrap_or("?"),
						f.source.as_deref().unwrap_or("?"),
						f.line
					)
				})
				.collect();
			let locals: Vec<String> = error
				.frames
				.iter()
				.find(|f| !f.locals.is_empty())
				.map(|f| {
					f.locals
						.iter()
						.take(12)
						.map(|l| {
							let v = match &l.value {
								ErrorValue::Integer(i) => i.to_string(),
								ErrorValue::Float(x) => x.to_string(),
								ErrorValue::String(b) => {
									String::from_utf8_lossy(b).chars().take(60).collect()
								}
								other => format!("{other:?}").chars().take(30).collect(),
							};
							format!("{}={v}", l.name)
						})
						.collect()
				})
				.unwrap_or_default();
			line(
				&log,
				&format!(
					"SCRIPT ERROR {message} | {} | locals: {}",
					stack.join(" < "),
					locals.join(", ")
				),
			);
		});
	}
	{
		let log = log.clone();
		let t0 = Instant::now();
		engine.set_log_handler(move |level, message| match level {
			LogLevel::Error | LogLevel::Warning => line(&log, &format!("[{level:?}] {message}")),
			_ if message.starts_with("WF ") || message.starts_with("PROBE") => line(&log, message),
			// Worldgen's progress, timed.
			_ if !message.starts_with("contract") && !message.starts_with("Executing") => line(
				&log,
				&format!("{:>8.2}s {message}", t0.elapsed().as_secs_f64()),
			),
			_ => {}
		});
	}
	harness::boot_menu(&engine)?;
	for _ in 0..60 {
		harness::run_frame(&engine, FRAME_MS)?;
	}
	if let Some(path) = &o.settings {
		let script =
			std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
		engine.run_script(&script, &path.to_string_lossy())?;
	}
	let eval = |expr: &str| {
		tactical_player::eval_i64(&engine, &format!("({expr}) ? 1 : 0"))
			.ok()
			.flatten()
			== Some(1)
	};
	let frames = |n: u32| -> Result<(), String> {
		for _ in 0..n {
			harness::run_frame(&engine, FRAME_MS)?;
		}
		Ok(())
	};
	engine.run_script(
		&format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"),
		"new campaign",
	)?;
	frames(1)?;
	let settings = format!(
		r#"{{ companyName = "BB Probe", banner = "banner_04", combatDifficulty = 1, economicDifficulty = 1,
		budgetDifficulty = 1, ironman = false, explorationDifficulty = false,
		lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
		seed = "{}", origin = "scenario.early_access" }}"#,
		o.seed
	);
	let mut profiler = o
		.profile_start
		.as_ref()
		.map(|_| squirrel::ScriptProfiler::new());
	let started = Instant::now();
	engine.run_script(
		&format!("{MENU}.getNewCampaignMenuModule().onStartButtonPressed({settings});"),
		"start",
	)?;
	let msu_settings = "(\"MSU\" in ::getroottable()) && ::MSU.SettingsScreen.isVisible()";
	let mut in_world = false;
	for _ in 0..3000 {
		if eval(IN_WORLD) {
			in_world = true;
			break;
		}
		if eval(msu_settings) {
			engine.run_script(
				"::MSU.SettingsScreen.onSaveButtonPressed({});",
				"MSU settings",
			)?;
		}
		match profiler.as_mut() {
			Some(p) => engine.vm().profiled(p, || frames(1))?,
			None => frames(1)?,
		}
	}
	if !in_world {
		return Err("the campaign didn't start in 3000 frames".into());
	}
	line(
		log,
		&format!(
			"campaign started in {:.2?} (worldgen included)",
			started.elapsed()
		),
	);
	if let (Some(p), Some(path)) = (profiler, &o.profile_start) {
		let report = bb_engine::script_profile::report(
			"campaign start",
			"",
			&p.profile(),
			&bb_engine::script_profile::Owners::of(&archives),
			25,
		);
		std::fs::write(path, report).map_err(|e| format!("{}: {e}", path.display()))?;
	}
	engine.run_script(include_str!("world_fights.nut"), "world_fights.nut")?;
	let say = |expr: &str| {
		tactical_player::eval_string(&engine, expr)
			.ok()
			.flatten()
			.unwrap_or_default()
	};
	line(
		log,
		&format!(
			"settlements {}",
			say("::World.EntityManager.getSettlements().len().tostring()")
		),
	);
	line(log, &format!("factions{}", say("::__wfFactions()")));
	let quiet = format!(
		"::__wfInstall(); ::World.State.getPlayer().setAttackable(false); ::World.State.m.IsAutosaving = false;
		if (::World.Assets.getMoney() < 50000) ::World.Assets.addMoney(100000);
		if (::World.Assets.getFood() < 60) {{ for (local i = 0; i < 4; ++i) ::World.Assets.getStash().add(::new(\"scripts/items/supplies/bread_item\")); ::World.Assets.updateFood(); }}
		::World.State.m.Events.update <- function(...) {{}};
		local s = ::World.State.getEventScreen(); if (s.isVisible() && !s.isAnimating()) s.onButtonPressed(0);
		if (::World.State.isPaused()) {{ ::World.State.setAutoPause(false); ::World.State.setPause(false); }}
		::World.setSpeedMult({});",
		o.speed
	);
	let day = || {
		tactical_player::eval_i64(&engine, "::World.getTime().Days")
			.ok()
			.flatten()
			.unwrap_or(-1)
	};
	if let Some(path) = &o.world_script {
		let script =
			std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
		engine.run_script(&quiet, "quiet")?;
		engine.run_script(&script, &path.to_string_lossy())?;
	}
	let start_day = day();
	let mut next_save = start_day + o.save_every_days;
	let mut next_status = start_day + 1;
	let mut saves = 0u32;
	let wall = Instant::now();
	let mut frame: u64 = 0;
	let mut slow_frames = 0u64;
	loop {
		if frame % 30 == 0 {
			engine.run_script(&quiet, "quiet")?;
		}
		let t = Instant::now();
		harness::run_frame(&engine, FRAME_MS)?;
		let dt = t.elapsed();
		if dt.as_millis() > 250 {
			slow_frames += 1;
			line(log, &format!("SLOW FRAME {dt:.2?} at day {}", day()));
		}
		frame += 1;
		if frame % 120 != 0 {
			continue;
		}
		let d = day();
		if d >= next_status {
			next_status = d + 1;
			line(
				log,
				&format!(
					"WF day {d} {} (wall {:.0?}, frame {frame})",
					say("::__wfStatus()"),
					wall.elapsed()
				),
			);
		}
		if o.save_every_days > 0 && d >= next_save {
			next_save = d + o.save_every_days;
			for k in 0..o.chain {
				saves += 1;
				let name = format!("bb_probe_wf_{saves}");
				let t = Instant::now();
				if let Err(e) =
					engine.run_script(&format!("::World.State.saveCampaign(\"{name}\");"), "save")
				{
					line(log, &format!("SAVE FAILED {name}: {e}"));
				}
				let size = std::fs::metadata(o.saves.join(format!("{name}.sav")))
					.map(|m| m.len())
					.unwrap_or(0);
				let saved = t.elapsed();
				let before = say("::__wfStatus()");
				if k > 0 {
					// `loadCampaign` ignores a load within 4 s (real time) of the last one: wait,
					// paused, so the saved fights stay as they were.
					engine.run_script("::World.State.setPause(true);", "pause")?;
					frames(250)?;
				}
				let t = Instant::now();
				engine.run_script(
					&format!("::World.State.campaign_menu_module_onLoadPressed(\"{name}\");"),
					"load",
				)?;
				// The load starts behind the loading screen: wait for it, then for the world.
				let mut loading = false;
				for _ in 0..600 {
					harness::run_frame(&engine, FRAME_MS)?;
					if eval("::LoadingScreen.isVisible()") {
						loading = true;
						break;
					}
				}
				let mut loaded = false;
				for _ in 0..6000 {
					if !loading {
						break;
					}
					harness::run_frame(&engine, FRAME_MS)?;
					if eval(IN_WORLD) {
						loaded = true;
						break;
					}
				}
				line(
					log,
					&format!(
						"SAVE/LOAD {name}: {size} bytes, saved in {saved:.2?}, loaded {loaded} in {:.2?}",
						t.elapsed()
					),
				);
				if !loaded {
					let state = say(
						"(::RootState.get(\"WorldState\") == null ? \"no WorldState\" : \"WorldState\") + \", loading screen \" + ::LoadingScreen.isVisible() + \", menu \" + (::RootState.get(\"MainMenuState\") != null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible()) + ((\"State\" in ::World) && ::World.State != null ? \", player \" + (::World.State.getPlayer() != null) + \", game over \" + ::World.State.m.IsGameOver : \", no World.State\")",
					);
					return Err(format!("loading {name} failed: {state}"));
				}
				engine.run_script(include_str!("world_fights.nut"), "world_fights.nut")?;
				line(log, &format!("  before: {before}"));
				line(log, &format!("  after:  {}", say("::__wfStatus()")));
				engine.run_script(&quiet, "quiet")?;
			}
		}
		if d - start_day >= o.days {
			break;
		}
	}
	line(
		log,
		&format!(
			"done: {} days, {frame} frames, {slow_frames} slow, {} script errors, wall {:.1?}",
			day() - start_day,
			errors.borrow(),
			wall.elapsed()
		),
	);
	Ok(())
}
