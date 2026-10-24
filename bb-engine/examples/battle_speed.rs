//! Fight one seeded battle at a chosen combat speed and frame rate, and report what the
//! battle drew and how it ended: to compare a battle at Swifter's combat speeds with the
//! same battle at 1x. The battle is set up as in `tests/combat_scenarios.rs` (a new
//! campaign, a `Const.World.Spawn` list offered with `startScriptedCombat`, the company
//! fought by the AI, the same pins: combat seed, wall-clock reseed, idle sounds, the turn
//! bar's battle end).
//!
//! ```text
//! battle_speed [--data <install>/data] [--spawn BanditRaiders] [--resources 200]
//!              [--faction Bandits] [--terrain tactical.plains] [--seed rust-engine-test]
//!              [--speed 1] [--fps 60] [--swifter] [--pin-render] [--draws FILE] [--script FILE]
//!              [--battle-seed 4242]
//! ```
//!
//! - `--speed` is the combat speed. Without `--swifter` it is applied as Swifter does it:
//!   the battle's virtual speed (`Time.setVirtualSpeed`, 1.5x that while everyone flees)
//!   and `Const.AI.Agent`'s four turn and evaluation delays divided by it. With
//!   `--swifter`, Modern Hooks, MSU and Swifter are mounted from the install's mods and
//!   Swifter's own `CombatSpeed` setting is set.
//! - `--battle-seed N` is the seed the battle starts from (the traces' 4242 by default).
//! - `--script FILE` runs a script on the world map before the battle is set up (e.g. to
//!   define a spawn list of its own for `--spawn`).
//! - `--draws FILE` writes every 2-arg `Math.rand` draw of the battle: its number, the
//!   running hash, the bounds, the result, the calling function and line, the virtual time,
//!   the round and the callers above it.
//! - Prints one line: draws, running hash, the result, rounds, casualties, the battle's
//!   virtual time and frames.

use std::path::PathBuf;

use bb_engine::harness;
use squirrel::{SqArray, SqTable};

struct Options {
	data: PathBuf,
	spawn: String,
	resources: i64,
	faction: String,
	terrain: String,
	seed: String,
	speed: f64,
	fps: f64,
	swifter: bool,
	pin_render: bool,
	draws: Option<PathBuf>,
	script: Option<PathBuf>,
	battle_seed: i64,
}

fn options() -> Result<Options, String> {
	let mut o = Options {
		data: PathBuf::from(harness::STEAM_DATA_DIR),
		spawn: "BanditRaiders".into(),
		resources: 200,
		faction: "Bandits".into(),
		terrain: "tactical.plains".into(),
		seed: "rust-engine-test".into(),
		speed: 1.0,
		fps: 60.0,
		swifter: false,
		pin_render: false,
		draws: None,
		script: None,
		battle_seed: 4242,
	};
	let mut args = std::env::args().skip(1);
	while let Some(a) = args.next() {
		let mut v = || args.next().ok_or(format!("{a} needs a value"));
		let num = |s: String| s.parse::<f64>().map_err(|e| format!("{a}: {e}"));
		match a.as_str() {
			"--data" => o.data = v()?.into(),
			"--spawn" => o.spawn = v()?,
			"--resources" => o.resources = num(v()?)? as i64,
			"--faction" => o.faction = v()?,
			"--terrain" => o.terrain = v()?,
			"--seed" => o.seed = v()?,
			"--speed" => o.speed = num(v()?)?,
			"--fps" => o.fps = num(v()?)?,
			"--swifter" => o.swifter = true,
			"--pin-render" => o.pin_render = true,
			"--draws" => o.draws = Some(v()?.into()),
			"--script" => o.script = Some(v()?.into()),
			"--battle-seed" => o.battle_seed = num(v()?)? as i64,
			_ => return Err(format!("unknown argument {a} (see the example's doc comment)")),
		}
	}
	Ok(o)
}

/// The trace (as `combat_scenarios.rs`'s, plus the draw log) and the speed.
const TRACE: &str = r#"
::__rng <- { n = 0, h = 0, on = false, pin = @pin@, log = [], t0 = 0.0 };
local origRand = ::Math.rand;
local origSeed = ::Math.seedRandom;
::Math.rand = function( ... )
{
	if (vargv.len() == 0) return origRand();
	local a = vargv[0], b = vargv[1];
	local t = ::__rng;
	if (t.on) ::__pinBar();
	if (t.on && t.pin) { local caller = ::getstackinfos(2); if (caller != null && caller.func == "onRender") return a; }
	local r = origRand(a, b);
	t.n++;
	t.h = (t.h * 33 + (r.tointeger() % 65521) + (a.tointeger() % 65521) * 3 + (b.tointeger() % 65521) * 5) % 1000003;
	if (t.h < 0) t.h += 1000003;
	if (t.on)
	{
		local c = ::getstackinfos(2);
		local chain = "";
		for (local l = 3; l < 7; l++) { local i = ::getstackinfos(l); if (i == null) break; chain += "<" + i.func + ":" + i.line; }
		local round = ("TurnSequenceBar" in ::Tactical) && ::Tactical.TurnSequenceBar != null ? ::Tactical.TurnSequenceBar.getCurrentRound() : -1;
		t.log.push(::format("%d %d %d %d %d %s:%d %.3f %d %s", t.n, t.h, a, b, r, c == null ? "?" : c.func, c == null ? 0 : c.line, ::Time.getVirtualTimeF() - t.t0, round, chain));
	}
	return r;
};
::Math.seedRandom = function( _s )
{
	if (::__rng.on && _s == ::Time.getRealTime()) _s = 999;
	return origSeed(_s);
};
::__pinBar <- function()
{
	if (!("State" in ::Tactical) || ::Tactical.State == null || ::Tactical.State.m.TacticalScreen == null) return;
	local bar = ::Tactical.State.m.TacticalScreen.m.TurnSequenceBar;
	if (bar == null || ("__pinned" in bar)) return;
	bar.__pinned <- true;
	local update = bar.update;
	bar.update <- function()
	{
		if (::Tactical.Entities.isCombatFinished()) this.m.IsBattleEnded = true;
		return update.call(this);
	};
};
::__speed <- { mult = @speed@, swifter = @swifter@, rounds = 0, vt = 0.0, frames = 0 };
if (!::__speed.swifter)
{
	// Swifter's CombatSpeed callback: the AI's turn and evaluation delays.
	foreach (key in [ "NewTurnDelayWithFasterMovement", "NewTurnDelay", "NewEvaluationDelay", "CameraAdditionalDelay" ])
		::Const.AI.Agent[key] = ::Math.round(::Const.AI.Agent[key] / ::__speed.mult);
}
// Every frame in a battle: Swifter's virtual speed where vanilla sets 1 (unpaused) or 1.5
// (everyone flees); and the battle's rounds and time.
local autoplay = ::__autoplay;
::__autoplay = function()
{
	if (("State" in ::Tactical) && ::Tactical.State != null && !::Tactical.State.isBattleEnded())
	{
		local s = ::__speed;
		if (!s.swifter && s.mult != 1.0)
		{
			local v = ::Time.getVirtualSpeed();
			if (v == 1.0) ::Time.setVirtualSpeed(s.mult);
			else if (v == 1.5) ::Time.setVirtualSpeed(1.5 * s.mult);
		}
		s.frames++;
		if (::__rng.on) s.vt = ::Time.getVirtualTimeF() - ::__rng.t0;
		if ("TurnSequenceBar" in ::Tactical && ::Tactical.TurnSequenceBar != null) s.rounds = ::Tactical.TurnSequenceBar.getCurrentRound();
	}
	autoplay();
};
::__combatTrace <- function()
{
	::World.State.m.IsAutosaving = false;
	if (::__speed.swifter) ::Swifter.Mod.ModSettings.getSetting("CombatSpeed").set(::__speed.mult);
	::Const.Sound.IdleSoundBaseDelay = 1.0e9;
	::Const.Sound.IdleSoundMinDelay = 1.0e9;
	local st = ::World.State.get();
	local orig = st.startScriptedCombat;
	st.startScriptedCombat <- function( ... )
	{
		if (vargv.len() == 0)
		{
			::Math.seedRandom(@seed@);
			this.m.CombatSeed = 777;
			::__rng.n = 0; ::__rng.h = 0; ::__rng.on = true;
			::__rng.t0 = ::Time.getVirtualTimeF();
		}
		local args = [this];
		args.extend(vargv);
		return orig.acall(args);
	};
	foreach (bro in ::World.getPlayerRoster().getAll())
	{
		local ocs = bro.onCombatStart;
		bro.onCombatStart <- function()
		{
			ocs.call(this);
			local a = ::new("scripts/ai/tactical/agents/military_melee_agent");
			a.setActor(this);
			this.setAIAgent(a);
			this.m.IsControlledByPlayer = false;
		};
		foreach (name in ["kill", "retreat", "onCombatFinished"])
		{
			local f = bro[name];
			bro[name] <- function( ... )
			{
				this.m.IsControlledByPlayer = true;
				local args = [this];
				args.extend(vargv);
				return f.acall(args);
			};
		}
	}
	::Math.seedRandom(@seed@);
	local p = st.getLocalCombatProperties(st.getPlayer().getPos(), true);
	p.Entities = [];
	p.Parties = [];
	p.TerrainTemplate = "@terrain@";
	p.CombatID = "Trace";
	::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.@spawn@, @resources@, ::World.FactionManager.getFactionOfType(::Const.FactionType.@faction@).getID());
	st.startScriptedCombat(p, true, true, true);
};
"#;

fn main() -> std::process::ExitCode {
	match options().and_then(|o| run(&o)) {
		Ok(()) => std::process::ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("battle_speed: {e}");
			std::process::ExitCode::FAILURE
		}
	}
}

fn run(o: &Options) -> Result<(), String> {
	let io = |e: std::io::Error| e.to_string();
	let mut archives = harness::data_archives(&o.data).map_err(io)?;
	if o.swifter {
		let mods = harness::mod_archives(&o.data).map_err(io)?;
		for prefix in ["mod_modern_hooks", "mod_msu", "mod_swifter"] {
			let m = mods
				.iter()
				.find(|m| m.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix)))
				.ok_or(format!("no {prefix} among the install's mods"))?;
			archives.push(m.clone());
		}
	}
	let engine = harness::open_mounted(&o.data, archives, |_| {}).map_err(io)?;
	harness::boot_menu(&engine)?;
	harness::prepare_new_campaign(&engine, &o.seed)?;
	harness::start_new_campaign(&engine)?;
	harness::install_autoplay(&engine)?;
	let trace = TRACE
		.replace("@pin@", if o.pin_render { "true" } else { "false" })
		.replace("@speed@", &format!("{:?}", o.speed))
		.replace("@swifter@", if o.swifter { "true" } else { "false" })
		.replace("@spawn@", &o.spawn)
		.replace("@resources@", &o.resources.to_string())
		.replace("@faction@", &o.faction)
		.replace("@terrain@", &o.terrain)
		.replace("@seed@", &o.battle_seed.to_string());
	if let Some(path) = &o.script {
		let script = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
		engine.run_script(&script, &path.to_string_lossy())?;
	}
	engine.run_script(&trace, "trace")?;
	let frame_ms = 1000.0 / o.fps;
	// As in the captures: ~2 s on the paused world, then the battle is offered and engaged.
	for _ in 0..(2.0 * o.fps) as u32 {
		harness::run_frame(&engine, frame_ms)?;
	}
	engine.run_script("::__autoplayEngage = true; ::__combatTrace();", "engage")?;
	let root = || SqTable::root(engine.vm());
	let mut done = false;
	for _ in 0..(2000.0 * o.fps) as u64 {
		harness::run_frame(&engine, frame_ms)?;
		if root().get::<i64>("__lastCombatResult").is_some() {
			done = true;
			break;
		}
	}
	let rng = root().get::<SqTable>("__rng").ok_or("no trace state")?;
	let speed = root().get::<SqTable>("__speed").ok_or("no speed state")?;
	let num = |t: &SqTable, k: &str| t.get::<f64>(k).unwrap_or(f64::NAN);
	if let Some(path) = &o.draws {
		let log = rng.get::<SqArray>("log").ok_or("no draw log")?;
		let lines: Vec<String> = (0..log.len()).filter_map(|i| log.get::<String>(i)).collect();
		std::fs::write(path, lines.join("\n") + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
	}
	let casualties = bb_engine::tactical_player::eval_i64(&engine, "::Tactical.getCasualtyRoster().getSize()")
		.ok()
		.flatten()
		.unwrap_or(-1);
	println!(
		"spawn {} speed {} fps {} done {done} draws {} hash {} result {} rounds {} casualties {casualties} virtual {:.2} frames {}",
		o.spawn,
		o.speed,
		o.fps,
		rng.get::<i64>("n").unwrap_or(-1),
		rng.get::<i64>("h").unwrap_or(-1),
		root().get::<i64>("__lastCombatResult").unwrap_or(-1),
		speed.get::<i64>("rounds").unwrap_or(-1),
		num(&speed, "vt"),
		speed.get::<i64>("frames").unwrap_or(-1),
	);
	Ok(())
}
