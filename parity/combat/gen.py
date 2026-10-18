#!/usr/bin/env python3
"""Draw-exact battle captures (bb-engine/tests/combat_scenarios.rs): build a
probe script for one scenario from the test's own TRACE and the harness's
AUTOPLAY (bb-engine/src/harness.rs), so the original runs exactly what our test runs.

    python3 parity/combat/gen.py NAME SPAWN RESOURCES FACTION TERRAIN [SETUP_FILE] [OUT_DIR]
    python3 "$BB_PROBE_RUNNER" OUT_DIR/NAME.nut \
        --mode campaign --seed rust-engine-test --out DIR

The probe engages the battle after 2 s of real time on the paused world (the test runs
120 frames), plays it out with the scripted player, and logs `PROBE marks` (draw count
and running hash every 2000 draws), `end` (the count and hash when the combat was first
finished), `result` and `after` (back on the world map: count, hash, casualties). A
scenario's `checkpoints` are the marks below `end` plus `end`; `after` and `result` go in
as they are. OUT_DIR defaults to `parity/combat/out` (git-ignored).
BB_PROBE_RUNNER is a probe runner that launches the original game (not part of this
repository).
"""
import re, sys, pathlib
ROOT = pathlib.Path(__file__).resolve().parents[2] / "modern_bb_engine"
def rust_const(path, name):
    s = (ROOT / path).read_text()
    m = re.search(r'const ' + name + r': &str = r#"(.*?)"#;', s, re.S)
    return m.group(1)
name, spawn, resources, faction, terrain = sys.argv[1:6]
setup = pathlib.Path(sys.argv[6]).read_text() if len(sys.argv) > 6 and sys.argv[6] else ""
out_dir = pathlib.Path(sys.argv[7]) if len(sys.argv) > 7 else pathlib.Path(__file__).parent / "out"
trace = (rust_const("bb-engine/tests/combat_scenarios.rs", "TRACE")
    .replace("@end@", "-1").replace("@pin@", "false").replace("@player@", "false")
    .replace("@setup@", setup).replace("@spawn@", spawn).replace("@resources@", resources)
    .replace("@faction@", faction).replace("@terrain@", terrain))
autoplay = rust_const("bb-engine/src/harness.rs", "AUTOPLAY")
driver = r'''
::BbProbe.DeferDone = true;
::__cap <- { ended = null, start = ::Time.getRealTimeF(), engaged = false };
::__capTick <- function( _ )
{
	local c = ::__cap;
	try
	{
		if (!c.engaged && ::Time.getRealTimeF() - c.start > 2.0)
		{
			c.engaged = true;
			::__autoplayEngage = true;
			::__combatTrace();
		}
		::__autoplay();
		if (c.ended == null && ("State" in ::Tactical) && ::Tactical.State != null && ::Tactical.Entities.isCombatFinished())
			c.ended = [::__rng.n, ::__rng.h];
		if ("after" in ::__rng)
		{
			local s = "";
			foreach (m in ::__rng.marks) s += m[0] + ":" + m[1] + " ";
			::logInfo("PROBE marks = " + s);
			::logInfo("PROBE end = " + (c.ended == null ? "none" : c.ended[0] + ":" + c.ended[1]));
			::logInfo("PROBE result = " + ("__lastCombatResult" in getroottable() ? ::__lastCombatResult : "none"));
			::logInfo("PROBE after = " + ::__rng.after[0] + ":" + ::__rng.after[1] + ":" + ::__rng.after[2]);
			::BbProbe.done();
			return;
		}
		if (::Time.getRealTimeF() - c.start > 330.0)
		{
			::logInfo("PROBE timeout = n " + ::__rng.n + " ended " + (c.ended != null));
			::BbProbe.done();
			return;
		}
	}
	catch (e) { ::logInfo("PROBE error = " + e); }
	::Time.scheduleEvent(::TimeUnit.Real, 16, ::__capTick, null);
};
::Time.scheduleEvent(::TimeUnit.Real, 16, ::__capTick, null);
'''
out_dir.mkdir(parents=True, exist_ok=True)
out = out_dir / (name + ".nut")
out.write_text(autoplay + "\n" + trace + "\n" + driver)
print(out)
