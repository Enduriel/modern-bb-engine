//! End-to-end: scripted battles against twenty-four kinds of enemies are **draw-exact**
//! against the original game, start to finish. From a new campaign (fixed seed), a battle against
//! one of vanilla's spawn lists is set up through `getLocalCombatProperties` +
//! `addUnitsToCombat` and offered with `startScriptedCombat`. The company fights it
//! through the AI (a `military_melee_agent` per brother, as a charm would), so both
//! sides' decisions run through the engine. The 2-arg `Math.rand` stream from the
//! battle's start (count and running hash, as in `combat_trace.rs`) must match the one
//! captured from the original (2026-10-13) at every checkpoint up to the last draw
//! before the battle ends, and the battle must end with
//! the same result. Where the capture has it, the return to the world map must match
//! too: the draws and casualties when the world's `onCombatFinished` has run (after the
//! result screen's Leave, pressed once, and the loading screen's tip).
//!
//! In the player battles the company is played by a scripted player instead
//! (`testdata/scripted_player.nut`, the same script the captures ran in the original):
//! its waits, skill clicks on targets, moves, inventory and End Turn go in through the
//! states' mouse and key input and the screens' callbacks, so those battles are
//! draw-exact through a player's input too.
//!
//! The checkpoints are the original's running hash, recomputed from its logged draws,
//! plus the end hash it logged itself. A logged bound can hide a float just below an
//! integer (`319` for 318.99…), which breaks the recompute from there on: such
//! checkpoints are left out.
//!
//! Pinned as in `combat_trace.rs`: the combat seed, the wall-clock reseed, idle sounds.
//! And the turn bar learns of the battle's end at once instead of on a 50 ms real-time
//! event, which otherwise lets the frame rate decide whether it starts another round
//! after the battle (a draw).
//! Ghosts also redraw their jitter in `onRender` whenever a sprite animation ends
//! (virtual time, so frame timing): with `pin_render`, draws made from `onRender` answer
//! without drawing, in both engines.
//! Forest maps take ~33,000 draws to generate; the fights add the rest.
//!
//! Skips if no Steam install. Slow in debug builds: each test runs all of worldgen first.

use std::path::Path;

use bb_engine::harness;
use squirrel::{SqArray, SqTable};

/// One battle: what is spawned, where, and what the original did.
struct Scenario {
	/// `Const.World.Spawn` list.
	spawn: &'static str,
	resources: i64,
	/// `Const.FactionType` the enemies belong to.
	faction: &'static str,
	terrain: &'static str,
	/// `(draw count, running hash)` in the original: every 2000 draws (where
	/// recomputable) and at the last draw before the battle ends.
	checkpoints: &'static [(i64, i64)],
	/// `Tactical.Entities.getCombatResult()` at the end.
	result: i64,
	/// Draws from `onRender` don't draw (see above).
	pin_render: bool,
	/// `(draw count, running hash, casualties)` once back on the world map, if captured.
	after: Option<(i64, i64, i64)>,
	/// The company is played by the scripted player (`testdata/scripted_player.nut`)
	/// instead of the AI.
	player: bool,
	/// Squirrel run on the world map before the battle (gear, the player's settings).
	setup: &'static str,
}

/// The trace: counts and hashes 2-arg draws and records the checkpoints, pins what
/// can't match, hands the company to the AI, and sets up and offers the battle
/// (`::__combatTrace()`). Integer arithmetic stays below 2^31 (the original's integers
/// are 32-bit).
const TRACE: &str = r#"
::__rng <- { n = 0, h = 0, on = false, end = @end@, marks = [], pin = @pin@ };
::__playerMode <- @player@;
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
	if (t.on && (t.n % 2000 == 0 || t.n == t.end)) t.marks.push([t.n, t.h]);
	return r;
};
::Math.seedRandom = function( _s )
{
	if (::__rng.on && _s == ::Time.getRealTime()) _s = 999;
	return origSeed(_s);
};
// The turn bar: told of the battle's end at once. (::Tactical.TurnSequenceBar is a
// WeakTableRef; the bar is the tactical screen's.)
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
::__combatTrace <- function()
{
	::World.State.m.IsAutosaving = false;
	@setup@
	if (::__playerMode)
	{
		// A real mouse has always told the cursor where it is (the capture's headless
		// game had not), and the scripted player plays the company.
		local vm = ::Settings.getVideoMode();
		::Cursor.setPosition(vm.Width / 2, vm.Height / 2);
		::__autoplayPlayer = ::__botTick;
	}
	::Const.Sound.IdleSoundBaseDelay = 1.0e9;
	::Const.Sound.IdleSoundMinDelay = 1.0e9;
	local st = ::World.State.get();
	// The battle itself starts with startScriptedCombat() (no args).
	local orig = st.startScriptedCombat;
	st.startScriptedCombat <- function( ... )
	{
		if (vargv.len() == 0)
		{
			::Math.seedRandom(4242);
			this.m.CombatSeed = 777;
			::__rng.n = 0; ::__rng.h = 0; ::__rng.on = true;
		}
		local args = [this];
		args.extend(vargv);
		return orig.acall(args);
	};
	// Back on the world map.
	local finished = st.onCombatFinished;
	st.onCombatFinished <- function( ... )
	{
		local args = [this];
		args.extend(vargv);
		local r = finished.acall(args);
		::__rng.after <- [::__rng.n, ::__rng.h, ::Tactical.getCasualtyRoster().getSize()];
		return r;
	};
	foreach (bro in (::__playerMode ? [] : ::World.getPlayerRoster().getAll()))
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
		// Back under the player's control for what checks it: death (roster removal),
		// retreat and the battle's end.
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
	::Math.seedRandom(4242);
	local p = st.getLocalCombatProperties(st.getPlayer().getPos(), true);
	p.Entities = [];
	p.Parties = [];
	p.TerrainTemplate = "@terrain@";
	p.CombatID = "Trace";
	::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.@spawn@, @resources@, ::World.FactionManager.getFactionOfType(::Const.FactionType.@faction@).getID());
	st.startScriptedCombat(p, true, true, true);
};
"#;

/// The scripted player of the player battles.
const SCRIPTED_PLAYER: &str = include_str!("testdata/scripted_player.nut");

fn fight(s: &Scenario) {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let end = s.checkpoints.last().expect("checkpoints").0;
	let trace = TRACE
		.replace("@end@", &end.to_string())
		.replace("@pin@", if s.pin_render { "true" } else { "false" })
		.replace("@player@", if s.player { "true" } else { "false" })
		.replace("@setup@", s.setup)
		.replace("@spawn@", s.spawn)
		.replace("@resources@", &s.resources.to_string())
		.replace("@faction@", s.faction)
		.replace("@terrain@", s.terrain);
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::install_autoplay(&engine).expect("autoplay");
	if s.player {
		engine.run_script(SCRIPTED_PLAYER, "scripted player").expect("scripted player");
	}
	engine.run_script(&trace, "trace").expect("trace");
	// As in the capture: ~2 s on the paused world, then the battle is offered and
	// engaged.
	for frame in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
	}
	engine.run_script("::__autoplayEngage = true; ::__combatTrace();", "engage").expect("engage");

	// Fight until the autoplay leaves the result screen.
	let root = || SqTable::root(engine.vm());
	for frame in 0..100_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("battle frame {frame}: {e}"));
		if root().get::<i64>("__lastCombatResult").is_some() {
			break;
		}
	}
	let rng = root().get::<SqTable>("__rng").expect("trace state");
	let marks = rng.get::<SqArray>("marks").expect("marks");
	let ours: Vec<(i64, i64)> = (0..marks.len())
		.map(|i| {
			let m = marks.get::<SqArray>(i).expect("mark");
			(m.get::<i64>(0).unwrap(), m.get::<i64>(1).unwrap())
		})
		.collect();
	for expected in s.checkpoints {
		assert!(ours.contains(expected), "checkpoint {expected:?} (ours: {ours:?})");
	}
	assert_eq!(root().get::<i64>("__lastCombatResult"), Some(s.result), "combat result");
	if let Some(after) = s.after {
		let back = |_: ()| root().get::<SqTable>("__rng").and_then(|t| t.get::<SqArray>("after"));
		for frame in 0..2000 {
			if back(()).is_some() {
				break;
			}
			harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
		}
		let a = back(()).expect("the world's onCombatFinished ran");
		let ours = (a.get::<i64>(0).unwrap(), a.get::<i64>(1).unwrap(), a.get::<i64>(2).unwrap());
		assert_eq!(ours, after, "back on the world map (draws, hash, casualties)");
	}
}

#[test]
fn direwolves_in_a_forest() {
	fight(&Scenario {
		spawn: "Direwolves",
		resources: 60,
		faction: "Beasts",
		terrain: "tactical.forest",
		checkpoints: &[
			(2000, 893634),
			(4000, 803576),
			(6000, 689700),
			(8000, 467933),
			(10000, 841645),
			(12000, 707946),
			(14000, 164004),
			(16000, 109235),
			(18000, 326271),
			(20000, 274510),
			(22000, 821709),
			(24000, 87646),
			(26000, 493294),
			(28000, 253438),
			(30000, 333177),
			(32000, 147238),
			(34000, 365740),
			(34026, 277739),
		],
		result: 4,
		pin_render: false,
		after: Some((34035, 928315, 3)),
		player: false,
		setup: "",
	});
}

#[test]
fn zombies_in_a_swamp() {
	fight(&Scenario {
		spawn: "Zombies",
		resources: 80,
		faction: "Zombies",
		terrain: "tactical.swamp",
		checkpoints: &[(2000, 221684), (4000, 414143), (6000, 490493), (8000, 964468), (10000, 845059), (11012, 654739)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn goblin_raiders_in_the_hills() {
	fight(&Scenario {
		spawn: "GoblinRaiders",
		resources: 60,
		faction: "Goblins",
		terrain: "tactical.hills",
		checkpoints: &[(2000, 816665), (4000, 403605), (6000, 4943), (8000, 79192), (8025, 319982)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn bandit_raiders_on_the_plains() {
	fight(&Scenario {
		spawn: "BanditRaiders",
		resources: 60,
		faction: "Bandits",
		terrain: "tactical.plains",
		// No 8000: the recompute breaks at draw 7833 (a bound of 318.99…).
		checkpoints: &[(2000, 385639), (4000, 271884), (6000, 98800), (8217, 683224)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn orc_raiders_on_the_steppe() {
	fight(&Scenario {
		spawn: "OrcRaiders",
		resources: 100,
		faction: "Orcs",
		terrain: "tactical.steppe",
		checkpoints: &[(4000, 233155), (7525, 620450)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn nomad_raiders_in_the_desert() {
	fight(&Scenario {
		spawn: "NomadRaiders",
		resources: 100,
		faction: "OrientalBandits",
		terrain: "tactical.desert",
		// The original's own end hash only (the recompute breaks on a float bound).
		checkpoints: &[(5238, 376116)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn alps_in_autumn_leaves() {
	fight(&Scenario {
		spawn: "Alps",
		resources: 60,
		faction: "Beasts",
		terrain: "tactical.forest_leaves",
		checkpoints: &[(32254, 39770)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn a_lindwurm_in_steppe_hills() {
	fight(&Scenario {
		spawn: "Lindwurm",
		resources: 60,
		faction: "Beasts",
		terrain: "tactical.hills_steppe",
		checkpoints: &[(4000, 223370), (7332, 453358)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn an_unhold_in_tundra_hills() {
	fight(&Scenario {
		spawn: "Unhold",
		resources: 60,
		faction: "Beasts",
		terrain: "tactical.hills_tundra",
		checkpoints: &[(4000, 483870), (6646, 590551)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn spiders_in_a_forest() {
	fight(&Scenario {
		spawn: "Spiders",
		resources: 60,
		faction: "Beasts",
		terrain: "tactical.forest",
		checkpoints: &[(16000, 109235), (34489, 96309)],
		result: 4,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn ghouls_in_autumn() {
	fight(&Scenario {
		spawn: "Ghouls",
		resources: 60,
		faction: "Beasts",
		terrain: "tactical.autumn",
		checkpoints: &[(4000, 779157), (7898, 720896)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn a_necromancer_in_the_tundra() {
	fight(&Scenario {
		spawn: "Necromancer",
		resources: 100,
		faction: "Zombies",
		terrain: "tactical.tundra",
		// The original's own end hash only (the recompute breaks on a float bound).
		checkpoints: &[(7487, 839311)],
		result: 4,
		pin_render: false,
		after: Some((7492, 208038, 1)),
		player: false,
		setup: "",
	});
}

#[test]
fn hexen_in_a_swamp() {
	fight(&Scenario {
		spawn: "HexenAndMore",
		resources: 80,
		faction: "Beasts",
		terrain: "tactical.swamp",
		checkpoints: &[(4000, 414143), (8000, 964468), (10476, 18822)],
		result: 4,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn a_schrat_in_a_snowy_forest() {
	fight(&Scenario {
		spawn: "Schrats",
		resources: 60,
		faction: "Beasts",
		terrain: "tactical.forest_snow",
		checkpoints: &[(4000, 279844), (12000, 197177), (25434, 721833)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn serpents_in_the_desert() {
	fight(&Scenario {
		spawn: "Serpents",
		resources: 60,
		faction: "Beasts",
		terrain: "tactical.desert",
		checkpoints: &[(4000, 329691), (4824, 57370)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn sand_golems_in_desert_hills() {
	fight(&Scenario {
		spawn: "SandGolems",
		resources: 60,
		faction: "Beasts",
		terrain: "tactical.hills_desert",
		checkpoints: &[(4000, 925438), (4577, 653931)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn barbarians_in_the_snow() {
	fight(&Scenario {
		spawn: "Barbarians",
		resources: 100,
		faction: "Barbarians",
		terrain: "tactical.snow",
		checkpoints: &[(4000, 307276), (6046, 206481)],
		result: 4,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn noble_troops_on_the_plains() {
	fight(&Scenario {
		spawn: "Noble",
		resources: 120,
		faction: "Bandits",
		terrain: "tactical.plains",
		// Fought as bandits (a noble house isn't hostile to a new company). The original's own end hash only (the recompute breaks on a float bound).
		checkpoints: &[(8090, 852691)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn southern_troops_in_desert_hills() {
	fight(&Scenario {
		spawn: "Southern",
		resources: 120,
		faction: "OrientalBandits",
		terrain: "tactical.hills_desert",
		// Fought as nomads (a city state isn't hostile). The original's own end hash only (the recompute breaks on a float bound).
		checkpoints: &[(5081, 584868)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn the_undead_army_in_the_tundra() {
	fight(&Scenario {
		spawn: "UndeadArmy",
		resources: 120,
		faction: "Undead",
		terrain: "tactical.tundra",
		// The original's own end hash only (the recompute breaks on a float bound).
		checkpoints: &[(7086, 834004)],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn ghosts_in_the_snow() {
	fight(&Scenario {
		spawn: "Ghosts",
		resources: 60,
		faction: "Undead",
		terrain: "tactical.snow",
		// The original's own end hash only (the recompute breaks on a float bound).
		checkpoints: &[(4986, 815848)],
		result: 3,
		pin_render: true,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn hyenas_in_the_steppe() {
	fight(&Scenario {
		spawn: "Hyenas",
		resources: 80,
		faction: "Beasts",
		terrain: "tactical.steppe",
		checkpoints: &[(2000, 122648), (4000, 233155), (6000, 962250), (7611, 177069)],
		result: 4,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn goblin_scouts_in_autumn_leaves() {
	fight(&Scenario {
		spawn: "GoblinScouts",
		resources: 100,
		faction: "Goblins",
		terrain: "tactical.forest_leaves",
		checkpoints: &[
			(2000, 765052),
			(4000, 927748),
			(6000, 697450),
			(8000, 107310),
			(10000, 74867),
			(12000, 162483),
			(14000, 801411),
			(16000, 422200),
			(18000, 204225),
			(20000, 74433),
			(22000, 708625),
			(24000, 287526),
			(26000, 895838),
			(28000, 515631),
			(30000, 286191),
			(32000, 730058),
			(32605, 909804),
		],
		result: 3,
		pin_render: false,
		after: None,
		player: false,
		setup: "",
	});
}

#[test]
fn a_goblin_boss_in_the_hills() {
	fight(&Scenario {
		spawn: "GoblinBoss",
		resources: 150,
		faction: "Goblins",
		terrain: "tactical.hills",
		checkpoints: &[(2000, 816665), (4000, 403605), (6000, 4943), (8000, 342239), (8818, 490959)],
		result: 3,
		pin_render: false,
		after: Some((8824, 941342, 1)),
		player: false,
		setup: "",
	});
}

/// Played by the scripted player: waits, clicks its skills on targets, walks up
/// to the enemy, ends its turns. The company falls.
#[test]
fn the_player_against_bandit_raiders_on_the_plains() {
	fight(&Scenario {
		spawn: "BanditRaiders",
		resources: 60,
		faction: "Bandits",
		terrain: "tactical.plains",
		checkpoints: &[(2000, 385639), (4000, 271884), (6000, 98800), (8000, 386271), (8603, 339247)],
		result: 4,
		pin_render: false,
		after: Some((8614, 587076, 3)),
		player: true,
		setup: "",
	});
}
