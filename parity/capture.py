#!/usr/bin/env python3
"""Capture parity goldens from the ORIGINAL game.

Groups the probes in parity/probes/ by (mode, seed) and runs each group as ONE game
launch through a probe runner that launches the original game (not part of this
repository; point BB_PROBE_RUNNER at it): the group's probes are concatenated in name order, each wrapped in a
function behind a `PROBE_SECTION <name>` marker and its own try/catch, so one failing
probe cannot hide the others. The PROBE lines are split back into one golden per probe
under parity/goldens/.

The UI payload probes in parity/ui_payloads/ (the scripts' side) each run
alone, with their sibling .js file driving the steps from the game's UI.

    python3 parity/capture.py                 # every probe
    python3 parity/capture.py menu/math       # only the groups containing these probes
    python3 parity/capture.py ui_payloads/world

The runner is expected to queue each launch until the game is free.
"""

import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def find_runner() -> Path:
    """The probe runner $BB_PROBE_RUNNER names (a script that launches the original game;
    not part of this repository)."""
    if not os.environ.get("BB_PROBE_RUNNER"):
        sys.exit("capture.py: set BB_PROBE_RUNNER to a probe runner that launches the original game "
                 "(not part of this repository)")
    return Path(os.environ["BB_PROBE_RUNNER"])


RUNNER = find_runner()

sys.path.insert(0, str(ROOT))
from probes import load_probes, combined_source, split_sections  # noqa: E402


def run_group(mode: str, seed: str | None, source: str, out_dir: Path, js: Path | None = None) -> list[str]:
    """Run one launch in the original; returns the PROBE log lines in order."""
    with tempfile.NamedTemporaryFile("w", suffix=".nut", delete=False) as f:
        f.write(source)
        probe_file = f.name
    cmd = [sys.executable, str(RUNNER), probe_file, "--mode", mode, "--out", str(out_dir)]
    if seed is not None:
        cmd += ["--seed", seed]
    if js is not None:
        cmd += ["--js", str(js)]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True)
    finally:
        os.unlink(probe_file)
    (out_dir / "runner.txt").write_text(result.stdout + result.stderr)
    if result.returncode != 0:
        sys.exit(f"runner failed for group {mode}/{seed} (see {out_dir}/runner.txt)")
    # Rows print as "[time] level tag text"; keep the probe rows' text.
    lines = []
    for row in result.stdout.splitlines():
        m = re.match(r"^\[[^\]]*\] \S+\s+\S+\s+(PROBE.*)$", row)
        if m:
            lines.append(m.group(1))
    return lines


def game_version(out_dir: Path) -> str:
    log = out_dir / "log.html"
    m = re.search(r"<title>([^<]*)</title>", log.read_text(errors="replace")) if log.exists() else None
    return m.group(1) if m else "unknown version"


def write_golden(name: str, lines: list[str], out_dir: Path):
    header = f"# captured {time.strftime('%Y-%m-%d')} from {game_version(out_dir)} (vanilla + Modern Hooks)\n"
    golden = ROOT / "goldens" / f"{name}.txt"
    golden.parent.mkdir(parents=True, exist_ok=True)
    golden.write_text(header + "".join(l + "\n" for l in lines))
    print(f"   {name}: {len(lines)} lines")


def main():
    if not RUNNER.exists():
        sys.exit(f"probe runner not found at {RUNNER} (BB_PROBE_RUNNER)")
    wanted = set(sys.argv[1:])
    groups = {}
    for probe in load_probes(ROOT / "probes"):
        groups.setdefault((probe.mode, probe.seed), []).append(probe)
    for (mode, seed), probes in sorted(groups.items(), key=lambda g: (g[0][0], g[0][1] or "")):
        if wanted and not wanted & {p.name for p in probes}:
            continue
        out_dir = Path(tempfile.mkdtemp(prefix=f"parity-{mode}-"))
        print(f"== {mode} (seed={seed}): {len(probes)} probes -> {out_dir}", flush=True)
        sections = split_sections(run_group(mode, seed, combined_source(probes), out_dir))
        for p in probes:
            got = sections.get(p.name)
            if got is None:
                print(f"   {p.name}: no output (section missing)")
                continue
            write_golden(p.name, got, out_dir)
    # UI payload probes: one launch each, as written, driven by their .js.
    for p in load_probes(ROOT / "ui_payloads", "ui_payloads/"):
        if wanted and p.name not in wanted:
            continue
        out_dir = Path(tempfile.mkdtemp(prefix="parity-ui-"))
        print(f"== {p.name} ({p.mode}, seed={p.seed}) -> {out_dir}", flush=True)
        js = (ROOT / p.name).with_suffix(".js")
        sections = split_sections(run_group(p.mode, p.seed, p.body, out_dir, js))
        got = sections.get(p.name)
        if got is None:
            print(f"   {p.name}: no output (section missing)")
            continue
        write_golden(p.name, got, out_dir)


if __name__ == "__main__":
    main()
