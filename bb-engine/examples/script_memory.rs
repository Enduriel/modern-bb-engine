//! What the script objects hold: boot the game headless, with the install's
//! mods or without, start a new campaign and load a save into it (or keep the new one), let the
//! world run, then write a census of the Squirrel VM's heap for
//! `parity/perf/script_memory.py`, and print the C heap in use.
//!
//! Run: `cargo run --release -p bb-engine --example script_memory -- [--mods]
//! [--saves DIR --save NAME] [--frames N] [--census FILE] [--track] [--play] [--dump FILE]
//! [--script FILE]`
//!
//! `--track` records, over the frames run after the load, which tables and keys the
//! scripts read, write and iterate, and which methods they fetch (the census lists them).
//! `--play` then plays a while: the world runs for 30 s, the company fights the weakest
//! hostile party (the AI on both sides), and the world runs 10 s more. `--dump` writes
//! every table and array the scripts can reach, slot by slot in iteration order, to
//! compare two runs (`BB_ENGINE_SEED` makes them repeatable). `--script` runs a script
//! (and a second of frames) before the census.

use std::path::{Path, PathBuf};

use bb_engine::{harness, tactical_player as player};

const LOADED: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";

/// The C heap's bytes in use (zero where the allocator doesn't say).
fn heap_mb() -> f64 {
	bb_engine::heap::bytes_in_use() as f64 / 1_048_576.0
}

struct Options {
	mods: bool,
	saves: Option<PathBuf>,
	save: Option<String>,
	frames: u32,
	census: Option<PathBuf>,
	track: bool,
	play: bool,
	dump: Option<PathBuf>,
	script: Option<PathBuf>,
}

fn options() -> Options {
	let mut o = Options { mods: false, saves: None, save: None, frames: 300, census: None, track: false, play: false, dump: None, script: None };
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().unwrap_or_else(|| panic!("{arg} needs a value"));
		match arg.as_str() {
			"--mods" => o.mods = true,
			"--saves" => o.saves = Some(PathBuf::from(value())),
			"--save" => o.save = Some(value()),
			"--frames" => o.frames = value().parse().expect("--frames N"),
			"--census" => o.census = Some(PathBuf::from(value())),
			"--track" => o.track = true,
			"--play" => o.play = true,
			"--dump" => o.dump = Some(PathBuf::from(value())),
			"--script" => o.script = Some(PathBuf::from(value())),
			other => panic!("unknown argument {other}"),
		}
	}
	o
}

fn frames(engine: &harness::GameEngine, n: u32) {
	for frame in 0..n {
		if let Err(e) = harness::run_frame(engine, 1000.0 / 60.0) {
			panic!("frame {frame}: {e}");
		}
	}
}

fn until(engine: &harness::GameEngine, max: u32, what: &str, cond: &str) {
	for _ in 0..max {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap() == Some(1) {
			return;
		}
		frames(engine, 1);
	}
	panic!("{what}: still not true after {max} frames");
}

fn main() {
	let o = options();
	let dir = Path::new(harness::STEAM_DATA_DIR);
	let start = std::time::Instant::now();
	let mut engine = if o.mods {
		harness::open_install_with_mods(dir).expect("open the install with its mods")
	} else {
		harness::open_install(dir).expect("open the install")
	};
	engine.vm_mut().install_standard_error_handlers();
	if let Some(saves) = &o.saves {
		engine.saves().set_directory(saves.clone());
	}
	harness::boot_menu(&engine).expect("boot the menu");
	frames(&engine, 60);
	println!("menu: {:.1} s, C heap {:.1} MB", start.elapsed().as_secs_f64(), heap_mb());
	new_campaign(&engine);
	if o.track {
		// From the load on: what the objects' own create() and deserialization write
		// counts as written.
		engine.vm().census_track(squirrel::CensusTrack::Start);
	}
	if let Some(name) = &o.save {
		// Loaded from inside the campaign, as the in-game menu's Load does (the world
		// state ignores a load within 4 s of the campaign's start).
		engine
			.run_script(
				&format!("::World.State.m.CampaignLoadTime = -100.0; ::World.State.loadCampaign(\"{name}\");"),
				"load",
			)
			.unwrap();
		frames(&engine, 2);
		until(&engine, 1200, "the campaign loading", LOADED);
	}
	frames(&engine, o.frames);
	if o.play {
		play(&engine);
	}
	if let Some(path) = &o.script {
		let src = std::fs::read_to_string(path).expect("read the script");
		engine.run_script(&src, &path.to_string_lossy()).expect("run the script");
		frames(&engine, 60);
	}
	let freed = engine.vm().collect_garbage();
	println!(
		"world: {:.1} s, C heap {:.1} MB (collector freed {freed})",
		start.elapsed().as_secs_f64(),
		heap_mb()
	);
	if let Some(path) = &o.dump {
		dump(&engine, path);
	}
	if let Some(path) = &o.census {
		engine.vm().write_census(path).expect("write the census");
		println!("census: {}", path.display());
	}
}

const UNPAUSE: &str = "if (::World.State.isPaused() && !::World.State.m.EventScreen.isVisible()) { ::World.State.setAutoPause(false); ::World.State.setPause(false); }";

/// The world for 30 s, a battle against the weakest hostile party, the world for 10 s.
fn play(engine: &harness::GameEngine) {
	harness::install_autoplay(engine).expect("autoplay");
	for _ in 0..30 {
		engine.run_script(UNPAUSE, "unpause").unwrap();
		frames(engine, 60);
	}
	engine
		.run_script(
			r#"
			local player = ::World.State.getPlayer();
			local target = null, weakest = 0;
			foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 100000.0))
			{
				if (!e.isParty() || e.getID() == player.getID() || e.isAlliedWithPlayer() || !e.isAttackable() || e.getTroops().len() == 0) continue;
				if (target == null || e.getStrength() < weakest) { target = e; weakest = e.getStrength(); }
			}
			if (target == null) throw "no hostile party";
			::logInfo("script_memory: fighting " + target.getName());
			foreach (bro in ::World.getPlayerRoster().getAll())
			{
				local b = bro.getBaseProperties();
				b.Hitpoints += 500;
				bro.getSkills().update();
				bro.setHitpoints(bro.getHitpointsMax());
			}
			local tp = target.getPos();
			player.setPos(::createVec(tp.X - 30, tp.Y));
			::World.setPlayerPos(player.getPos());
			::__autoplayEngage = true;
			::__autoplayFight = true;
			::__autoplayDieAsPlayers = true;
			::World.State.m.AutoAttack = ::WeakTableRef(target);
			"#,
			"fight",
		)
		.expect("pick a fight");
	let in_battle = "(\"State\" in ::Tactical) && ::Tactical.State != null";
	for _ in 0..120 {
		if player::eval_i64(engine, &format!("({in_battle}) ? 1 : 0")).unwrap() == Some(1) {
			break;
		}
		engine.run_script(UNPAUSE, "unpause").unwrap();
		frames(engine, 60);
	}
	println!("battle started: {}", player::eval_i64(engine, &format!("({in_battle}) ? 1 : 0")).unwrap() == Some(1));
	until(engine, 60_000, "the battle ends", "::RootState.get(\"TacticalFromWorldState\") == null && (\"__lastCombatResult\" in ::getroottable())");
	engine.run_script("::__autoplayEngage = false; ::__autoplayFight = false;", "no more battles").unwrap();
	until(engine, 600, "back on the world map", "::World.State.isVisible() && !::LoadingScreen.isVisible()");
	for _ in 0..10 {
		engine.run_script(UNPAUSE, "unpause").unwrap();
		frames(engine, 60);
	}
}

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";

/// Main menu → New Campaign → Start, as a player does (MSU's settings page kept as is).
fn new_campaign(engine: &harness::GameEngine) {
	until(engine, 60, "main menu", &format!("{MENU}.isVisible() && !{MENU}.isAnimating()"));
	engine.run_script(&format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"), "new campaign").unwrap();
	frames(engine, 1);
	engine
		.run_script(
			&format!(
				"{MENU}.getNewCampaignMenuModule().onStartButtonPressed({{
					companyName = \"Census Company\", banner = \"banner_04\",
					combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
					ironman = false, explorationDifficulty = false,
					lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
					seed = \"rust-engine-test\", origin = \"scenario.early_access\"
				}});"
			),
			"start",
		)
		.unwrap();
	for _ in 0..600 {
		if player::eval_i64(engine, &format!("({LOADED}) ? 1 : 0")).unwrap() == Some(1) {
			return;
		}
		if player::eval_i64(engine, "((\"MSU\" in ::getroottable()) && ::MSU.SettingsScreen.isVisible()) ? 1 : 0").unwrap() == Some(1) {
			engine.run_script("::MSU.SettingsScreen.onSaveButtonPressed({});", "MSU settings").unwrap();
		}
		frames(engine, 1);
	}
	panic!("the campaign didn't start");
}

/// Every table and array reachable from the root, in iteration order, one per line.
fn dump(engine: &harness::GameEngine, path: &Path) {
	const SCRIPT: &str = include_str!("script_memory_dump.nut");
	engine.run_script(SCRIPT, "dump").expect("dump the heap");
	let lines = squirrel::SqTable::root(engine.vm()).get::<squirrel::SqArray>("__dump").expect("the dump");
	let mut out = String::new();
	for i in 0..lines.len() {
		out.push_str(&lines.get::<String>(i).unwrap_or_default());
		out.push('\n');
	}
	std::fs::write(path, out).expect("write the dump");
	engine.run_script("::__dump = null;", "drop the dump").unwrap();
}
