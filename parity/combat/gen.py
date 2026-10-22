#!/usr/bin/env python3
"""Draw-exact battle captures (bb-engine/tests/combat_scenarios.rs): build a
probe script for one scenario from the test's own TRACE and the harness's
AUTOPLAY (bb-engine/src/harness.rs), so the original runs exactly what our test runs.

    python3 parity/combat/gen.py NAME SPAWN RESOURCES FACTION TERRAIN [SETUP_FILE] [OUT_DIR]
    python3 "$BB_PROBE_RUNNER" OUT_DIR/NAME.nut \
        --mode campaign --seed rust-engine-test --timeout 1200 --out DIR

The probe engages the battle after 2 s of real time on the paused world (the test runs
120 frames), plays it out with the scripted player, and logs `PROBE marks` (draw count
and running hash every 2000 draws), `end` (the count and hash when the combat was first
finished), `result` and `after` (back on the world map: count, hash, casualties). A
scenario's `checkpoints` are the marks below `end` plus `end`; `after` and `result` go in
as they are. OUT_DIR defaults to `parity/combat/out` (git-ignored).
BB_PROBE_RUNNER is a probe runner that launches the original game (not part of this
repository).
"""
import os, re, sys, pathlib
ROOT = pathlib.Path(__file__).resolve().parents[2]
def rust_const(path, name):
    s = (ROOT / path).read_text()
    m = re.search(r'const ' + name + r': &str = r#"(.*?)"#;', s, re.S)
    return m.group(1)
name, spawn, resources, faction, terrain = sys.argv[1:6]
setup = pathlib.Path(sys.argv[6]).read_text() if len(sys.argv) > 6 and sys.argv[6] else ""
out_dir = pathlib.Path(sys.argv[7]) if len(sys.argv) > 7 else pathlib.Path(__file__).parent / "out"
# PIN_RENDER=1: draws from onRender answer without drawing (the scenario's pin_render).
pin = "true" if os.environ.get("PIN_RENDER") == "1" else "false"
trace = (rust_const("bb-engine/tests/combat_scenarios.rs", "TRACE")
    .replace("@end@", "-1").replace("@pin@", pin).replace("@player@", "false")
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
		// The scenario's intro event (1 s in, later in a slow start) is closed first, and
		// the battle offered a second after; at 15 s it's offered regardless. The battle
		// reseeds the stream as it starts, so none of this reaches the trace.
		local ev = ::World.State.getEventScreen(), now = ::Time.getRealTimeF() - c.start;
		if (!c.engaged && ev.isVisible())
		{
			::World.State.m.MenuStack.popAll(true);
			c.closedAt <- now;
		}
		else if (!c.engaged && (("closedAt" in c) && now - c.closedAt > 1.0 || now > 15.0))
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
		if (::Time.getRealTimeF() - c.start > (("lastNote" in c) ? c.lastNote + 30.0 : 30.0))
		{
			c.lastNote <- ::Time.getRealTimeF() - c.start;
			local tac = ("State" in ::Tactical) && ::Tactical.State != null;
			::logInfo("probe-progress t " + c.lastNote.tointeger() + " n " + ::__rng.n + " engaged " + c.engaged + " tactical " + tac + (tac ? " round " + ::Tactical.TurnSequenceBar.getCurrentRound() : ""));
		}
		if (::Time.getRealTimeF() - c.start > 1150.0)
		{
			local w = "n " + ::__rng.n + " ended " + (c.ended != null) + " loading " + ::LoadingScreen.isVisible();
			try
			{
				w += " tactical " + (("State" in ::Tactical) && ::Tactical.State != null);
				if (("State" in ::Tactical) && ::Tactical.State != null)
				{
					local st = ::Tactical.State, a = ::Tactical.TurnSequenceBar.getActiveEntity();
					w += " backsteps " + st.m.MenuStack.hasBacksteps() + " paused " + st.isPaused() + " inputLocked " + st.isInputLocked();
					w += " round " + ::Tactical.TurnSequenceBar.getCurrentRound() + " active " + (a == null ? "none" : a.getName() + " player " + a.isPlayerControlled() + " turn " + a.isTurnStarted());
					if (a != null && a.getAIAgent() != null) w += " agent finished " + a.getAIAgent().isFinished() + " evaluating " + a.getAIAgent().isEvaluating();
				}
				else
				{
					local ws = ::World.State;
					w += " combatDialog " + (ws.m.CombatDialog != null && ws.m.CombatDialog.isVisible()) + " event " + ws.getEventScreen().isVisible();
				}
			}
			catch (e) { w += " (" + e + ")"; }
			::logInfo("PROBE timeout = " + w);
			::BbProbe.done();
			return;
		}
	}
	catch (e) { ::logInfo("PROBE error = " + e); }
	::Time.scheduleEvent(::TimeUnit.Real, 16, ::__capTick, null);
};
::Time.scheduleEvent(::TimeUnit.Real, 16, ::__capTick, null);
// A retreat clears every scheduled event (tactical_state's flee): the tick goes on.
local clearEvents = ::Time.clearEvents;
::Time.clearEvents = function( ... )
{
	local args = [this];
	args.extend(vargv);
	local r = clearEvents.acall(args);
	::Time.scheduleEvent(::TimeUnit.Real, 16, ::__capTick, null);
	return r;
};
'''
# LOG_DRAWS=FROM:TO also logs each draw in that window (to find where two runs part).
window = os.environ.get("LOG_DRAWS")
if window:
    lo, hi = window.split(":")
    driver += r'''
local traced = ::Math.rand;
::Math.rand = function( ... )
{
	local args = [this];
	args.extend(vargv);
	local r = traced.acall(args);
	local n = ::__rng.n;
	if (vargv.len() == 2 && n >= @lo@ && n <= @hi@)
	{
		local w = "";
		for (local l = 2; l < 5; l++)
		{
			local i = ::getstackinfos(l);
			if (i == null) break;
			w += (l > 2 ? "/" : "") + i.func + ":" + i.line;
		}
		local who = "";
		try
		{
			local i = ::getstackinfos(2), loc = i.locals;
			foreach (k in ["_entity", "_target", "target"])
				if (k in loc && loc[k] != null && typeof loc[k] != "integer")
				{
					local e = loc[k], t = e.getTile();
					who += " " + k + "=" + e.getName() + "@" + t.SquareCoords.X + "," + t.SquareCoords.Y + (k != "_entity" ? " vis " + t.IsVisibleForEntity : "");
				}
		}
		catch (err) { who += " (" + err + ")"; }
		if (who == "")
		{
			local i = ::getstackinfos(2);
			if (i != null && "locals" in i) foreach (k, v in i.locals) who += " " + k + ":" + typeof v;
			else who = " no locals";
		}
		::logInfo("PROBE d" + n + " = " + vargv[0] + "," + vargv[1] + "=" + r + " " + w + who);
	}
	return r;
};
'''.replace("@lo@", lo).replace("@hi@", hi)
out_dir.mkdir(parents=True, exist_ok=True)
out = out_dir / (name + ".nut")
out.write_text(autoplay + "\n" + trace + "\n" + driver)
print(out)
