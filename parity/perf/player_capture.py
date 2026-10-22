#!/usr/bin/env python3
"""Write a `.bat` a Windows player double-clicks to capture how bb's frames and the UI page's
paints go on their machine, comparable with a `frame_breakdown.py` run here.

    python3 parity/perf/player_capture.py OUT.bat

The `.bat` sits beside `bb.exe` and runs it through a fixed minute, with no input needed: the
main menu (still, then a full-view repaint by the page's scripts every animation frame, then a
full-view CSS fade: `frame_breakdown.py`'s `ui repaint` and `ui fade`), a new campaign
(vanilla, `--no-mods`, a seed of its own), its character screen (still, then the repaint),
then it quits. It writes `BB_FRAME_TIMES`, `BB_STATS`'s lines and the logs into
`bb-perf-capture` beside `bb.exe`; the campaign's saves go there too (`--saves`), and Steam
isn't touched (`--no-steam`). Each phase logs `PHASE name t=..`; `page_capture.py` reads the
folder. The `--at` timeline assumes 144 frames a second; at a slower rate it just takes longer.
"""
import sys

HZ = 144  # frames a second the timeline assumes (slower displays just take longer)
PHASE = '::logInfo("PHASE {name} t=" + ::Time.getRealTimeF());'
START = ('::RootState.get("MainMenuState").campaign_menu_module_onStartPressed({ Name = "Probe Company", '
         'Banner = ::Const.PlayerBanners[0], Difficulty = 1, EconomicDifficulty = 1, BudgetDifficulty = 1, '
         'Ironman = false, ExplorationMode = false, GreaterEvil = ::Const.World.GreaterEvilType.Random, '
         'PermanentDestruction = false, Seed = "rust-engine-test", StartingScenario = "scenario.early_access" });')
CLOSE_EVENT = 'local s = ::World.State.getEventScreen(); if (s != null && s.isVisible()) s.onButtonPressed(0);'
SETTLE = '::World.State.m.IsAutosaving = false; ::World.State.getMenuStack().popAll(true); ::World.State.setPause(true);'
CHARACTER = '::World.State.showCharacterScreen();'
EXIT = '::World.State.m.IsAutosaving = false; ::World.State.getMenuStack().popAll(true); ::World.State.exitGame();'
QUIT = '::RootState.get("MainMenuState").main_menu_module_onQuitPressed();'
REPAINT_ON = ("(function () { var d = document.createElement('div'); d.id = 'probe-repaint'; "
              "d.style.cssText = 'position:fixed;left:0;top:0;width:100vw;height:100vh;z-index:99999;pointer-events:none;"
              "font:20px serif;color:#fff;overflow:hidden;opacity:0.5'; document.body.appendChild(d); var n = 0; "
              "function step() { if (!document.getElementById('probe-repaint')) return; n++; "
              "d.style.background = 'hsl(' + (n * 7 % 360) + ',40%,30%)'; "
              "d.textContent = ('frame ' + n + ' ').repeat(400); requestAnimationFrame(step); } "
              "requestAnimationFrame(step); return 1; })()")
REPAINT_OFF = "(function () { var d = document.getElementById('probe-repaint'); if (d) d.remove(); return 1; })()"
FADE_ON = ("(function () { var d = document.createElement('div'); d.id = 'probe-fade'; "
           "d.style.cssText = 'position:fixed;left:0;top:0;width:100vw;height:100vh;background:#203040;z-index:99999;"
           "pointer-events:none;animation:probefade 1s ease-in-out infinite alternate'; "
           "var s = document.createElement('style'); s.id = 'probe-fade-style'; "
           "s.textContent = '@keyframes probefade { from { opacity: 0 } to { opacity: 0.8 } }'; "
           "document.head.appendChild(s); document.body.appendChild(d); return 1; })()")
FADE_OFF = ("(function () { ['probe-fade', 'probe-fade-style'].forEach(function (id) { "
            "var e = document.getElementById(id); if (e) e.remove(); }); return 1; })()")

at, at_page = [], []
def sq(secs, code): at.append((int(secs * HZ), code))
def js(secs, code): at_page.append((int(secs * HZ), code))
def phase(secs, name): sq(secs, PHASE.format(name=name))

# Each phase's line is logged as its change is made (`page_capture.py` leaves out its first second).
phase(5, "menu")
js(13, REPAINT_ON); phase(13, "menu repaint")
js(21, REPAINT_OFF); js(21, FADE_ON); phase(21, "menu fade")
js(29, FADE_OFF); phase(29, "load"); sq(29, START)
sq(38, CLOSE_EVENT)
sq(41, SETTLE); sq(41.5, CHARACTER); phase(41.5, "character screen")
js(51, REPAINT_ON); phase(51, "character repaint")
js(59, REPAINT_OFF); phase(59, "end"); sq(60, EXIT)
sq(64, QUIT)

def arg(text):
    """One argument as cmd passes it on and Rust's (MSVC's) parser reads it: in double quotes,
    inner double quotes as \\", % doubled (a .bat would expand it)."""
    # cmd reads the inner quotes as ending and starting its quoting, so what stands between
    # them mustn't be special to it.
    for part in text.split('"')[1::2]:
        assert not set('^&|<>') & set(part), part
    return '"' + text.replace('"', '\\"').replace('%', '%%') + '"'

args = []
for f, code in at:
    args += ["--at", str(f), arg(code)]
for f, code in at_page:
    args += ["--at-page", str(f), arg(code)]

lines = [
    "@echo off",
    "rem Battle Brothers (modern engine): a one-minute performance capture.",
    "rem Put this file next to bb.exe and double-click it. The game starts, plays itself",
    "rem through the main menu and the character screen of a new test campaign, and closes.",
    "rem Don't touch the mouse or keyboard meanwhile. The results go in the folder",
    "rem bb-perf-capture next to bb.exe: zip that folder and send it.",
    "setlocal",
    'set "OUT=%~dp0bb-perf-capture"',
    'if exist "%OUT%" rmdir /s /q "%OUT%"',
    'mkdir "%OUT%"',
    'set "BB_FRAME_TIMES=%OUT%\\frames.txt"',
    "set BB_STATS=1",
    "echo Capturing... the game closes by itself in about a minute.",
    '"%~dp0bb.exe" --no-steam --mute --no-mods --saves "%OUT%\\saves" --logs "%OUT%\\logs" '
    + " ".join(args) + ' > "%OUT%\\output.txt" 2>&1',
    "echo Done. Please zip the folder %OUT% and send it.",
    "pause",
]
out = sys.argv[1]
# cmd's command line takes at most 8191 characters.
assert len(lines[-3]) < 8000, len(lines[-3])
with open(out, "w", newline="\r\n") as f:
    f.write("\n".join(lines) + "\n")
