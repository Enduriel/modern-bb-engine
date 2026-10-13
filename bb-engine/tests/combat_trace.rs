//! End-to-end: a battle is **draw-exact** against the original game. From a new
//! campaign (fixed seed), the player engages the nearest hostile party; both engines
//! reseed at `startCombat` and the battle is fought out (the AI fights; the player's
//! units end their turns at once). The 2-arg `Math.rand` stream from there (count and a
//! running hash of every draw's bounds and result) must match the one captured from the
//! original (2026-10-13) at every 500th draw, through the
//! map generation, the spawns, and two rounds of AI turns (30 moves, 24 attacks).
//!
//! Three things can't match between runs and are pinned in both: the combat seed (from
//! the C runtime's `rand()`), the wall-clock reseed at the end of the spawn, and idle
//! sounds (fired on virtual-time gaps between turns, i.e. on frame timing).
//!
//! Skips if no Steam install. Slow in debug builds: it runs all of worldgen first.

use std::path::Path;

use bb_engine::harness;
use squirrel::{SqArray, SqTable};

/// `(draw count, running hash)` in the original, every 500 draws and at the end of the
/// capture.
const ORIGINAL: [(i64, i64); 13] = [
	(500, 767915),
	(1000, 420725),
	(1500, 382054),
	(2000, 213169),
	(2500, 753887),
	(3000, 310466),
	(3500, 430782),
	(4000, 610926),
	(4500, 27652),
	(5000, 219220),
	(5500, 125439),
	(6000, 461115),
	(6325, 540615),
];

/// The trace (the same script the original was captured with, minus its logging):
/// counts and hashes 2-arg draws, records the checkpoints, pins what can't match, and
/// offers the battle (`::__combatTrace()`). Integer arithmetic stays below 2^31 (the
/// original's integers are 32-bit).
const TRACE: &str = r#"
::__rng <- { n = 0, h = 0, on = false, marks = [] };
local origRand = ::Math.rand;
local origSeed = ::Math.seedRandom;
::Math.rand = function( ... )
{
	if (vargv.len() == 0) return origRand();
	local a = vargv[0], b = vargv[1];
	local r = origRand(a, b);
	local t = ::__rng;
	t.n++;
	t.h = (t.h * 33 + (r.tointeger() % 65521) + (a.tointeger() % 65521) * 3 + (b.tointeger() % 65521) * 5) % 1000003;
	if (t.h < 0) t.h += 1000003;
	if (t.on && (t.n % 500 == 0 || t.n == 6325)) t.marks.push([t.n, t.h]);
	return r;
};
::Math.seedRandom = function( _s )
{
	if (::__rng.on && _s == ::Time.getRealTime()) _s = 999;
	return origSeed(_s);
};
::__combatTrace <- function()
{
	::Const.Sound.IdleSoundBaseDelay = 1.0e9;
	::Const.Sound.IdleSoundMinDelay = 1.0e9;
	local st = ::World.State.get();
	foreach (name in ["startCombat", "startScriptedCombat"])
	{
		local orig = st[name];
		st[name] <- function( ... )
		{
			::Math.seedRandom(4242);
			this.m.CombatSeed = 777;
			::__rng.n = 0; ::__rng.h = 0; ::__rng.on = true;
			local args = [this];
			args.extend(vargv);
			return orig.acall(args);
		};
	}
	local player = st.getPlayer();
	local target = null; local best = 0;
	foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 100000.0))
	{
		if (!e.isParty() || e.getID() == player.getID() || e.isAlliedWithPlayer() || e.getTroops().len() == 0) continue;
		local d = e.getDistanceTo(player);
		if (target == null || d < best) { target = e; best = d; }
	}
	local tp = target.getPos();
	player.setPos(::createVec(tp.X - 30, tp.Y));
	::World.setPlayerPos(player.getPos());
	st.showCombatDialog(true, true, true, null, player.getPos());
};
"#;

#[test]
fn a_battle_is_draw_exact_against_the_original() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::install_autoplay(&engine).expect("autoplay");
	engine.run_script(TRACE, "trace").expect("trace");
	// As in the capture: ~2 s on the paused world (the scenario's intro event fires
	// after 1 s), then the battle is offered and engaged.
	for frame in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
	}
	engine.run_script("::__autoplayEngage = true; ::__combatTrace();", "engage").expect("engage");

	let draws = || SqTable::root(engine.vm()).get::<SqTable>("__rng").and_then(|t| t.get::<i64>("n")).unwrap_or(0);
	for frame in 0..20_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("battle frame {frame}: {e}"));
		if draws() >= 6325 {
			break;
		}
	}
	let rng = SqTable::root(engine.vm()).get::<SqTable>("__rng").expect("trace state");
	let marks = rng.get::<SqArray>("marks").expect("marks");
	let ours: Vec<(i64, i64)> = (0..marks.len())
		.map(|i| {
			let m = marks.get::<SqArray>(i).expect("mark");
			(m.get::<i64>(0).unwrap(), m.get::<i64>(1).unwrap())
		})
		.collect();
	for (i, expected) in ORIGINAL.iter().enumerate() {
		assert_eq!(ours.get(i), Some(expected), "checkpoint {i} (ours: {ours:?})");
	}
}
