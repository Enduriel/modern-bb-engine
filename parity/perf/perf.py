#!/usr/bin/env python3
"""Performance baselines from the ORIGINAL game.

Runs three launches through the probe runner (BB_PROBE_RUNNER) and writes parity/perf/original.txt:
- menu mode: the time to the main menu, the memory there, and the script benches
  (benches.nut), timed with the game's live clock;
- campaign mode (seed rust-engine-test): the new campaign's time, the memory once the
  world is up, the world benches (benches_world.nut), the frame rate on the running
  world map, and save/load times. The save is "bb_probe_perf"; it is deleted after.
- battle mode (a campaign, same seed): a battle's start, from the combat dialog offered
  to the battle running behind no loading screen (original_battle.nut; autosaving off).
  A launch of its own: the original crashes running more after a load made from a timed
  event, as the campaign mode's is.

While the game runs, its resident memory (VmRSS of BattleBrothers.exe, the Wine process,
so Wine's own share is included) is sampled every 0.25 s and matched to the probe's
marks by the log's wall-clock timestamps (1 s resolution).

    python3 parity/perf/perf.py              # every launch
    python3 parity/perf/perf.py menu         # one of them

Our engine's side: `cargo run --release -p bb-engine --example perf_baseline`, which
prints both columns (headless), and `bb-app`'s `load_times` example (the game with its
renderer and page: boot, new campaign, load, a battle's start).
"""

import datetime
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent))
from capture import RUNNER, game_version  # noqa: E402

OUT = ROOT / "original.txt"
SEED = "rust-engine-test"
SAVE_NAME = "bb_probe_perf"
SAVE_DIR = Path.home() / ".local/share/Steam/steamapps/compatdata/365360/pfx/drive_c/users/steamuser/Documents/Battle Brothers/savegames"


def game_pids() -> list[int]:
    pids = []
    for d in Path("/proc").iterdir():
        if d.name.isdigit():
            try:
                if (d / "comm").read_text().startswith("BattleBrothers"):
                    pids.append(int(d.name))
            except OSError:
                pass
    return pids


def status_kb(pid: int, field: str) -> int | None:
    try:
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith(field + ":"):
                return int(line.split()[1])
    except OSError:
        pass
    return None


class Sampler(threading.Thread):
    """VmRSS of the game process every 0.25 s, from start() until stop()."""

    def __init__(self):
        super().__init__(daemon=True)
        self.samples: list[tuple[float, int]] = []  # (wall time, kB)
        self.peak_kb = 0
        self.first_seen: float | None = None
        self.load: list[float] = []  # 1-minute load averages, sampled each second
        self._stop = threading.Event()

    def run(self):
        while not self._stop.is_set():
            for pid in game_pids():
                rss = status_kb(pid, "VmRSS")
                hwm = status_kb(pid, "VmHWM")
                if rss is not None:
                    if self.first_seen is None:
                        self.first_seen = time.time()
                    self.samples.append((time.time(), rss))
                if hwm is not None:
                    self.peak_kb = max(self.peak_kb, hwm)
            if len(self.samples) % 4 == 0:
                self.load.append(os.getloadavg()[0])
            self._stop.wait(0.25)

    def stop(self):
        self._stop.set()
        self.join()

    def at(self, wall: float) -> int | None:
        """The largest sample in the log's second `wall` (its rows have 1 s resolution)."""
        inside = [kb for t, kb in self.samples if wall <= t < wall + 1.0]
        return max(inside) if inside else None


def run(mode: str, source: str, setup: str | None = None) -> tuple[list[tuple[str, str]], Sampler, Path]:
    """One launch; returns its (row time, PROBE text) rows, the sampler and the out dir."""
    out_dir = Path(tempfile.mkdtemp(prefix=f"perf-{mode}-"))
    files = []
    def temp(text: str) -> str:
        f = tempfile.NamedTemporaryFile("w", suffix=".nut", delete=False)
        f.write(text)
        f.close()
        files.append(f.name)
        return f.name
    runner_mode = "campaign" if mode in ("campaign", "battle") else mode
    cmd = [sys.executable, "-u", str(RUNNER), temp(source), "--mode", runner_mode, "--out", str(out_dir)]
    if runner_mode == "campaign":
        cmd += ["--seed", SEED, "--setup", temp(setup)]
    print(f"== {mode} -> {out_dir}", flush=True)
    sampler = Sampler()
    rows, log = [], []
    try:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        for line in proc.stdout:
            log.append(line)
            if line.startswith("launched"):
                sampler.start()
            elif line.startswith("=== outcome") and sampler.is_alive():
                sampler.stop()
            m = re.match(r"^\[([0-9:]+)\] \S+\s+\S+\s+(PROBE.*)$", line.rstrip("\n"))
            if m:
                rows.append((m.group(1), m.group(2)))
        proc.wait()
    finally:
        for f in files:
            os.unlink(f)
        if sampler.is_alive():
            sampler.stop()
    (out_dir / "runner.txt").write_text("".join(log))
    if proc.returncode != 0:
        sys.exit(f"runner failed for {mode} (see {out_dir}/runner.txt)")
    return rows, sampler, out_dir


def row_wall(hms: str, near: float) -> float:
    """A log row's HH:MM:SS as wall time, on the day of `near`."""
    day = datetime.datetime.fromtimestamp(near).date()
    t = datetime.datetime.combine(day, datetime.time.fromisoformat(hms)).timestamp()
    return t - 86400 if t > near + 43200 else t


def results(rows, sampler: Sampler) -> dict[str, str]:
    out = {}
    for hms, text in rows:
        m = re.match(r"^PROBE perf (.+?) = (.*)$", text)
        if not m:
            continue
        key, value = m.group(1), m.group(2)
        if key.startswith("mark "):
            name = key[len("mark "):]
            wall = row_wall(hms, sampler.first_seen or time.time())
            kb = sampler.at(wall)
            if kb is not None:
                out[f"rss_mb at {name}"] = f"{kb / 1024:.0f}"
            if name == "menu" and sampler.first_seen is not None:
                out["launch_to_menu_s"] = f"{wall - sampler.first_seen:.0f}"
            continue
        out[key] = value
    out["rss_mb peak"] = f"{sampler.peak_kb / 1024:.0f}"
    if sampler.load:
        # Other work on the machine skews the timings: record the load.
        out["machine loadavg_1m max"] = f"{max(sampler.load):.1f}"
    return out


def delete_probe_save():
    for f in SAVE_DIR.glob(SAVE_NAME + "*"):
        f.unlink()
        print(f"   deleted {f.name}")


def cpu_model() -> str:
    for line in Path("/proc/cpuinfo").read_text().splitlines():
        if line.startswith("model name"):
            return line.split(":", 1)[1].strip()
    return "unknown CPU"


def main():
    if not RUNNER.exists():
        sys.exit(f"probe runner not found at {RUNNER} (BB_PROBE_RUNNER)")
    modes = sys.argv[1:] or ["menu", "campaign", "battle"]
    read = lambda name: (ROOT / name).read_text()
    old = {}
    if OUT.exists():
        for line in OUT.read_text().splitlines():
            if " = " in line and not line.startswith("#"):
                k, v = line.split(" = ", 1)
                old[k] = v
    new, version = {}, "unknown version"
    if "menu" in modes:
        rows, sampler, out_dir = run("menu", read("benches.nut") + read("original_common.nut") + read("original_menu.nut"))
        new.update({f"menu {k}": v for k, v in results(rows, sampler).items()})
        version = game_version(out_dir)
    if "campaign" in modes:
        try:
            rows, sampler, out_dir = run("campaign",
                                         read("benches_world.nut") + read("original_common.nut") + read("original_campaign.nut"),
                                         read("original_campaign_setup.nut"))
        finally:
            delete_probe_save()
        new.update({f"campaign {k}": v for k, v in results(rows, sampler).items()})
        version = game_version(out_dir)
    if "battle" in modes:
        rows, sampler, out_dir = run("battle", read("original_common.nut") + read("original_battle.nut"),
                                     read("original_campaign_setup.nut"))
        new.update({f"battle {k}": v for k, v in results(rows, sampler).items()})
        version = game_version(out_dir)
    merged = {k: v for k, v in old.items() if k.split(" ", 1)[0] not in modes}
    merged.update(new)
    header = (f"# The original's performance baselines, from parity/perf/perf.py.\n"
              f"# captured {time.strftime('%Y-%m-%d')} from {version} (vanilla + Modern Hooks, Proton,\n"
              f"# headless gamescope) on {cpu_model()} ({os.cpu_count()} threads). Times in ms\n"
              f"# unless the key says s;"
              f" memory is the Wine process's resident set.\n")
    OUT.write_text(header + "".join(f"{k} = {v}\n" for k, v in sorted(merged.items())))
    print(f"wrote {OUT}")
    for k, v in sorted(new.items()):
        print(f"   {k} = {v}")


if __name__ == "__main__":
    main()
