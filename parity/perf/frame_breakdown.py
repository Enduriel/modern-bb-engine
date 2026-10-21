#!/usr/bin/env python3
"""Play `bb` through a fixed list of scenes in a headless gamescope and measure each:
per-frame spans (`BB_FRAME_TIMES`), per-thread CPU and wake-ups and every process's
memory (`proc_sample.py`), and optionally the passes' GPU times (`BB_GPU_TIMES`), the
scripts' profile (`BB_SCRIPT_PROFILE`) or a Chromium trace of the page (`cdp_trace.py`).

    python3 parity/perf/frame_breakdown.py --bin BB --out DIR [--vanilla] [--save FILE]
        [--hz 180] [--size 2560x1358] [--vsync on|off] [--quota 400] [--phase-secs 8]
        [--gpu-times] [--script-profile] [--trace] [--cdp-memory] [--only PHASE,...] [--battles N]
        [--day SECS] [--clear-weather]

- The modded run (the default) loads a copy of `--save` (default: the newest save in
  bb's savegames folder) named `bb_probe_*`, with the install's mods; `--vanilla`
  starts a new campaign without mods. The saves themselves are only read.
- Every bb runs inside `gamescope --backend headless` (nothing on the desktop), muted,
  under `systemd-run --user --scope -p CPUQuota=<quota>% -p MemoryMax=8G`, behind the shared game queue
  (`/tmp/bb-probe-queue`); only its own processes are killed.
- The scenes, in order: main menu; load (or the new campaign); the world map paused at
  zoom 1, zoomed out, panned at zoom 1 and zoomed out; running at 1x and 4x; a town;
  the character screen; a full-view CSS fade injected into the page over the paused map;
  a battle (its start, the player's turn idle, its menu open, the character screen open,
  the player's turn in a storm's rain, its menu open in the rain, AI turns). `--day SECS` adds a phase running at
  4x for SECS after the 4x one (28 s crosses a whole day: dusk and dawn).
  `--battles N` then fights N battles by the AI (memory after each). Each scene logs `PHASE name` and lasts
  `--phase-secs`; the driver samples /proc in its middle.

Writes DIR/frames.txt, DIR/phases.json (frame ranges, for `frame_times.py`),
DIR/samples.json (per phase: proc_sample's output), DIR/stderr.txt, DIR/logs, with
`--trace` DIR/trace-PHASE.txt (Chromium's trace and a JS profile), with `--cdp-memory`
DIR/cdp-memory-PHASE.txt (Chromium's memory-infra dump); then prints `frame_times.py`'s summary.
"""
import json, os, re, shutil, signal, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import proc_sample  # noqa: E402

SAVES = os.path.expanduser("~/.local/share/battle-brothers-modern/savegames")
QUEUE = "/tmp/bb-probe-queue"
DEVTOOLS = 9333

MENU = '::RootState.get("MainMenuState").m.MainMenuScreen'
MSU = ('::__msu <- function( _d ) { if (("MSU" in ::getroottable()) && ::MSU.SettingsScreen.isVisible()) '
       '{ ::MSU.SettingsScreen.onSaveButtonPressed({}); return; } ::Time.scheduleEvent(::TimeUnit.Real, 250, ::__msu, null); }; '
       '::Time.scheduleEvent(::TimeUnit.Real, 250, ::__msu, null);')
PHASE = '::logInfo("PHASE {name} t=" + ::Time.getRealTimeF());'
SETTLE = ('::World.State.m.IsAutosaving = false; ::World.State.getMenuStack().popAll(true); ::World.State.setPause(true); '
          '::World.getCamera().Zoom = 1.0; ::__c <- ::World.State.getPlayer().getPos();')
CLEAR_HOSTILES = ('local p = ::World.State.getPlayer(); p.m.IsAttackable = false; p.m.IsAttackableByAI = false; '
                  'foreach (e in ::World.getAllEntitiesAtPos(p.getPos(), 800.0)) '
                  'if (e.isParty() && e.getID() != p.getID() && !e.isAlliedWithPlayer()) e.die();')
TOWN = ('::World.State.getMenuStack().popAll(true); local state = ::World.State; local tile = state.getPlayer().getTile(); '
        'local nearest = null; local distance = 0; foreach (s in ::World.EntityManager.getSettlements()) '
        '{ if (!s.isEnterable()) continue; local d = s.getTile().getDistanceTo(tile); '
        'if (nearest == null || d < distance) { nearest = s; distance = d; } } state.enterLocation(nearest);')
ENGAGE = ('local player = ::World.State.getPlayer(); player.m.IsAttackable = true; local target = null; local best = 0; '
          'foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 100000.0)) '
          '{ if (!e.isParty() || e.getID() == player.getID() || e.isAlliedWithPlayer() || e.getTroops().len() == 0) continue; '
          'local d = e.getDistanceTo(player); if (target == null || d < best) { target = e; best = d; } } '
          '::logInfo("engaging " + target.getName() + " (" + target.getTroops().len() + ")"); local tp = target.getPos(); '
          'player.setPos(::createVec(tp.X - 30, tp.Y)); ::World.setPlayerPos(player.getPos()); '
          '::World.State.showCombatDialog(true, true, true, null, player.getPos());')
CONFIRM = 'if (::World.State.m.CombatDialog != null && ::World.State.m.CombatDialog.isVisible()) ::World.State.m.CombatDialog.onEngageButtonPressed();'
ACTIVE = ('local a = ::Tactical.TurnSequenceBar.getActiveEntity(); '
          '::logInfo("battle active: " + (a == null ? "none" : a.getName() + (a.isPlayerControlled() ? " (player)" : " (AI)")));')
END_PLAYER_TURN = ('if (("State" in ::Tactical) && ::Tactical.State != null && !::Tactical.State.isBattleEnded()) '
                   '{ local a = ::Tactical.TurnSequenceBar.getActiveEntity(); '
                   'if (a != null && a.isPlayerControlled()) ::Tactical.TurnSequenceBar.onNextTurnButtonPressed(); }')
# A storm's rain over the battle (`tactical_entity_manager.setupWeather`'s), the player's turn still idle.
RAIN = ('local w = ::Tactical.getWeather(); local r = w.createRainSettings(); r.MinDrops = 150; r.MaxDrops = 150; '
        'r.NumSplats = 50; r.MinVelocity = 400.0; r.MaxVelocity = 500.0; r.MinAlpha = 1.0; r.MaxAlpha = 1.0; '
        'r.MinScale = 0.75; r.MaxScale = 1.0; w.buildRain(r); ::logInfo("rain built");')
START_CAMPAIGN = ('::RootState.get("MainMenuState").campaign_menu_module_onStartPressed({ Name = "Probe Company", '
                  'Banner = ::Const.PlayerBanners[0], Difficulty = 1, EconomicDifficulty = 1, BudgetDifficulty = 1, '
                  'Ironman = false, ExplorationMode = false, GreaterEvil = ::Const.World.GreaterEvilType.Random, '
                  'PermanentDestruction = false, Seed = "rust-engine-test", StartingScenario = "scenario.early_access" });')
CLOSE_EVENT = 'local s = ::World.State.getEventScreen(); if (s != null && s.isVisible()) s.onButtonPressed(0);'
FADE_ON = ("(() => { const d = document.createElement('div'); d.id = 'probe-fade'; "
           "d.style.cssText = 'position:fixed;left:0;top:0;width:100vw;height:100vh;background:#203040;z-index:99999;"
           "pointer-events:none;animation:probefade 1s ease-in-out infinite alternate'; "
           "const s = document.createElement('style'); s.id = 'probe-fade-style'; "
           "s.textContent = '@keyframes probefade { from { opacity: 0 } to { opacity: 0.8 } }'; "
           "document.head.appendChild(s); document.body.appendChild(d); return 1; })()")
FADE_OFF = "(() => { for (const id of ['probe-fade', 'probe-fade-style']) document.getElementById(id)?.remove(); return 1; })()"


def opt(args, name, default=None):
    return args[args.index(name) + 1] if name in args else default


def newest_save():
    saves = [f for f in os.listdir(SAVES) if f.endswith(".sav") and not f.startswith(("MSU#", "bb_probe_"))]
    return max(saves, key=lambda f: os.path.getmtime(os.path.join(SAVES, f)))


class Timeline:
    """The `--at` scripts and the phases' frame ranges."""

    def __init__(self, rate):
        self.rate, self.frame, self.at, self.at_page, self.phases = rate, 0, [], [], []

    def f(self, secs):
        return int(secs * self.rate)

    def run(self, code, after=0.0):
        self.at.append((self.frame + self.f(after), code))

    def run_page(self, code):
        """Run `code` (JS) in the page at this frame (bb's `--at-page`)."""
        self.at_page.append((self.frame, code))

    def wait(self, secs):
        self.frame += self.f(secs)

    def phase(self, name, secs, setup="", every_frame=None):
        if setup:
            self.run(setup)
        self.run(PHASE.format(name=name))
        start = self.frame
        if every_frame:
            for i in range(self.f(secs)):
                self.at.append((start + i + 1, every_frame(i)))
        self.wait(secs)
        self.phases.append([name, start + 1, self.frame])


def pan(reach):
    def code(i):
        import math
        x = reach * math.sin(2 * math.pi * i / 480)
        y = reach * 0.5 * math.cos(2 * math.pi * i / 720)
        return f"::World.getCamera().setPos(::createVec(::__c.X + {x:.1f}, ::__c.Y + {y:.1f}));"
    return code


def build(args, rate):
    vanilla, secs = "--vanilla" in args, float(opt(args, "--phase-secs", 8))
    only = opt(args, "--only")
    want = (lambda n: True) if not only else (lambda n: any(n.startswith(o) for o in only.split(",")))
    t = Timeline(rate)
    if not vanilla:
        t.run(MSU, after=1.0)
    t.wait(4)
    t.phase("menu", secs)
    if vanilla:
        t.run(PHASE.format(name="load"))
        t.run(START_CAMPAIGN)
        t.run(CLOSE_EVENT, after=8)
    else:
        t.run(PHASE.format(name="load"))
        t.run(f"{MENU}.getMainMenuModule().onLoadCampaignButtonPressed();")
        t.run(f'{MENU}.getLoadCampaignMenuModule().onLoadButtonPressed("bb_probe_save");', after=0.3)
    t.wait(10)
    t.phase("world paused zoom 1", secs, SETTLE)
    if want("world paused zoomed out"):
        t.phase("world paused zoomed out", secs, "::World.getCamera().Zoom = 9.77;")
    if want("world pan"):
        t.phase("world pan zoom 1", secs, "::World.getCamera().Zoom = 1.0;", pan(1500.0))
        t.phase("world pan zoomed out", secs, "::World.getCamera().Zoom = 9.77;", pan(6000.0))
    t.run("::World.getCamera().Zoom = 1.0; ::World.getCamera().setPos(::__c);")
    t.wait(0.5)
    if want("world run"):
        t.phase("world run 1x", secs, CLEAR_HOSTILES + " ::World.State.setAutoPause(false); ::World.State.setPause(false); ::World.setSpeedMult(1.0);")
        t.phase("world run 4x", secs, "::World.setSpeedMult(4.0);")
        day = float(opt(args, "--day", 0))
        if day:
            # A whole day at 4x (105 s of game time is 26 s): its dusk and dawn.
            t.phase("world run 4x day", day)
        t.run("::World.State.setPause(true);")
        t.wait(0.5)
    if want("town"):
        t.run(TOWN)
        t.wait(2)
        t.phase("town", secs)
        t.run("::World.State.getMenuStack().popAll(true); ::World.State.setPause(true);")
        t.wait(2)
    if want("character"):
        t.run("::World.State.showCharacterScreen();")
        t.wait(2)
        t.phase("character screen", secs)
        t.run("::World.State.getMenuStack().popAll(true); ::World.State.setPause(true);")
        t.wait(2)
    if want("ui fade"):
        t.run_page(FADE_ON)
        t.wait(0.5)
        t.phase("ui fade", secs)
        t.run_page(FADE_OFF)
        t.wait(1)
    if want("battle"):
        t.run(ENGAGE)
        t.run(CONFIRM, after=1.5)
        t.wait(9)
        # `--clear-weather`: no rain or snow (their fades run on real time, so a battle
        # with them changes even while paused).
        start = ACTIVE + (" ::Tactical.getWeather().clear();" if "--clear-weather" in args else "")
        t.phase("battle start", secs, start)
        t.phase("battle player idle", secs, ACTIVE)
        t.phase("battle menu", secs, "::Tactical.State.toggleMenuScreen();")
        t.run("::Tactical.State.toggleMenuScreen();")
        t.wait(1)
        t.phase("battle character screen", secs, "::Tactical.State.showCharacterScreen();")
        t.run("::Tactical.State.hideCharacterScreen();")
        t.wait(1)
        t.phase("battle rain", secs, RAIN)
        # The storm with the battle paused (its menu): the rain's fades go on.
        t.phase("battle rain menu", secs, "::Tactical.State.toggleMenuScreen();")
        t.run("::Tactical.State.toggleMenuScreen();")
        t.wait(1)
        t.phase("battle ai turns", secs * 2, ACTIVE, lambda i: END_PLAYER_TURN if i % 30 == 0 else None)
    battles = int(opt(args, "--battles", 0))
    if battles:
        t.run(AUTOPLAY_DRIVER)
        for b in range(battles):
            if b:
                t.run(ENGAGE)
            t.run("::__autoplayFight = true; ::__autoplayDieAsPlayers = true; ::__autoplayEngage = true;")
            t.wait(150)
            t.phase(f"after battle {b + 1}", secs, '::logInfo("battle over: " + ((("State" in ::Tactical) && ::Tactical.State != null) ? "no, still fighting" : "yes, result " + (("__lastCombatResult" in ::getroottable()) ? ::__lastCombatResult : "?"))); if (::World.State != null) ::World.State.setPause(true);')
    t.wait(1)
    t.run('::logInfo("PHASE end");')
    t.at = [(f, c) for f, c in t.at if c]
    return t


# The harness's scripted player (bb-engine harness.rs AUTOPLAY), run every 100 ms of real time.
AUTOPLAY_DRIVER = None


def autoplay_driver():
    src = open(os.path.join(HERE, "../../modern_bb_engine/bb-engine/src/harness.rs")).read()
    body = src.split('const AUTOPLAY: &str = r#"', 1)[1].split('"#;', 1)[0]
    return body + ('\n::__autoplayTick <- function( _d ) { try { ::__autoplay(); } catch (e) { ::logInfo("autoplay: " + e); } '
                   '::Time.scheduleEvent(::TimeUnit.Real, 100, ::__autoplayTick, null); };\n'
                   '::Time.scheduleEvent(::TimeUnit.Real, 100, ::__autoplayTick, null);\n')


def take_ticket():
    os.makedirs(QUEUE, exist_ok=True)
    ticket = os.path.join(QUEUE, f"{time.time_ns():020d}-{os.getpid()}")
    open(ticket, "w").close()

    def live():
        out = []
        for n in os.listdir(QUEUE):
            try:
                pid = int(n.split("-", 1)[1])
            except ValueError:
                continue
            if os.path.exists(f"/proc/{pid}"):
                out.append(os.path.join(QUEUE, n))
            else:
                try:
                    os.remove(os.path.join(QUEUE, n))
                except OSError:
                    pass
        return sorted(out)

    while live()[:1] != [ticket]:
        time.sleep(1)
    return ticket


def main():
    global AUTOPLAY_DRIVER
    args = sys.argv[1:]
    binary, out = opt(args, "--bin"), opt(args, "--out")
    vanilla = "--vanilla" in args
    hz = int(opt(args, "--hz", 180))
    width, height = opt(args, "--size", "2560x1358").split("x")
    vsync = opt(args, "--vsync", "on") == "on"
    quota = opt(args, "--quota", "400")
    secs = float(opt(args, "--phase-secs", 8))
    rate = hz if vsync else 144
    AUTOPLAY_DRIVER = autoplay_driver()
    os.makedirs(out, exist_ok=True)
    saves, logs = os.path.join(out, "saves"), os.path.join(out, "logs")
    os.makedirs(saves, exist_ok=True)
    os.makedirs(logs, exist_ok=True)
    if not vanilla:
        save = opt(args, "--save") or newest_save()
        shutil.copy(os.path.join(SAVES, save), os.path.join(saves, "bb_probe_save.sav"))
        for f in os.listdir(SAVES):
            if f.startswith("MSU#"):
                shutil.copy(os.path.join(SAVES, f), os.path.join(saves, f))
    settings = os.path.join(out, "settings.cfg")
    with open(settings, "w") as f:
        f.write(f"video.width = {width}\nvideo.height = {height}\nvideo.color_bits = 32\nvideo.refresh_rate = {hz}\n"
                f"video.fullscreen = false\nvideo.borderless = false\nvideo.vsync = {'true' if vsync else 'false'}\n"
                "video.ui_scale = 1\nvideo.scene_scale = 1\nvideo.depth_of_field = false\nvideo.gamma = 1\nvideo.brightness = 1\n")
    t = build(args, rate)
    with open(os.path.join(out, "phases.json"), "w") as f:
        json.dump(t.phases, f)
    bb = [binary, "--mute", "--no-steam", "--settings", settings, "--saves", saves, "--logs", logs]
    # DevTools only for what reads the page through them: a page served to DevTools never
    # rests between frames (bb's `BB_PAGE_REST`), so it would change what's measured.
    if "--trace" in args or "--cdp-memory" in args:
        bb += ["--devtools", str(DEVTOOLS)]
    if not vanilla:
        bb.append("--mods")
    for frame, code in t.at:
        bb += ["--at", str(frame), code]
    for frame, code in t.at_page:
        bb += ["--at-page", str(frame), code]
    env = dict(os.environ, BB_FRAME_TIMES=os.path.join(out, "frames.txt"), BB_STATS="1")
    if "--gpu-times" in args:
        env.update(BB_GPU_TIMES="1")
    if "--script-profile" in args:
        env.update(BB_SCRIPT_PROFILE=os.path.join(out, "script_profile.md"), BB_SCRIPT_PROFILE_SECS="2")
    cmd = ["systemd-run", "--user", "--scope", "-q", "-p", f"CPUQuota={quota}%", "-p", "MemoryMax=8G", "gamescope", "--backend", "headless",
           "-W", width, "-H", height, "-w", width, "-h", height, "-r", str(hz), "--"] + bb
    ticket = take_ticket()
    samples = {}
    try:
        err = open(os.path.join(out, "stderr.txt"), "w")
        p = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace",
                             start_new_session=True)
        pending = []
        threads = []

        def measure(name):
            time.sleep(1.5)
            root = proc_sample.newest("bb")
            if root is None:
                return
            if "--trace" in args and name not in ("load",):
                import cdp_trace
                import contextlib, io
                buf = io.StringIO()
                try:
                    trace = cdp_trace.record(DEVTOOLS, max(1.0, secs - 6), None)
                    with contextlib.redirect_stdout(buf):
                        cdp_trace.analyze(trace, None)
                except Exception as e:  # noqa: BLE001
                    buf.write(f"trace failed: {e}\n")
                try:
                    with contextlib.redirect_stdout(buf):
                        cdp_trace.js_profile(DEVTOOLS, 2.0)
                except Exception as e:  # noqa: BLE001
                    buf.write(f"JS profile failed: {e}\n")
                with open(os.path.join(out, f"trace-{name.replace(' ', '_')}.txt"), "w") as f:
                    f.write(buf.getvalue())
            else:
                samples[name] = proc_sample.sample(root, max(1.0, secs - 3.5), True)
            if "--cdp-memory" in args:
                import contextlib, io
                import cdp_memory
                buf = io.StringIO()
                try:
                    with contextlib.redirect_stdout(buf):
                        sys.argv = ["cdp_memory.py", str(DEVTOOLS)]
                        cdp_memory.main()
                except Exception as e:  # noqa: BLE001
                    buf.write(f"memory dump failed: {e}\n")
                with open(os.path.join(out, f"cdp-memory-{name.replace(' ', '_')}.txt"), "w") as f:
                    f.write(buf.getvalue())

        deadline = time.time() + float(opt(args, "--timeout", 900))
        for line in p.stdout:
            err.write(line)
            m = re.search(r"PHASE (.+?) t=", line) or re.search(r"PHASE (end)", line)
            if m:
                name = m.group(1)
                if name == "end":
                    break
                if name != "load":
                    th = threading.Thread(target=measure, args=(name,))
                    th.start()
                    threads.append(th)
            if time.time() > deadline:
                err.write("frame_breakdown: timed out\n")
                break
        for th in threads:
            th.join()
        os.killpg(p.pid, signal.SIGTERM)
        try:
            p.wait(timeout=20)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGKILL)
            p.wait()
        err.close()
    finally:
        os.remove(ticket)
    with open(os.path.join(out, "samples.json"), "w") as f:
        json.dump(samples, f)
    subprocess.run([sys.executable, os.path.join(HERE, "frame_times.py"), os.path.join(out, "frames.txt"),
                    os.path.join(out, "phases.json"), "--budget", f"{1000 / rate:.3f}", "--min-share", "0.05"],
                   stdout=open(os.path.join(out, "summary.md"), "w"))
    for name, s in samples.items():
        print(f"## {name}")
        proc_sample.report(s)


if __name__ == "__main__":
    main()
