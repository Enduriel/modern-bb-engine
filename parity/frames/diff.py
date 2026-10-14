#!/usr/bin/env python3
"""Diff every scene of ours against the original's (frame parity).

    python3 parity/frames/diff.py OURS ORIGINAL OUT [--original2 DIR] [--ours2 DIR]

OURS and ORIGINAL hold one PNG per scene (frame_scenes' output and the probe's
--shots). With a second run of the original (--original2) and of ours (--ours2), what
differs between an engine's own two runs is left out as noise. Writes, per scene,
OUT/<scene>.png (the heatmap) and OUT/<scene>-region-NN.png (ours | original), and
OUT/summary.txt; prints the summary.
"""

import argparse
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DIFF = ROOT / "modern_bb_engine" / "target" / "release" / "examples" / "frame_diff"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ours", type=Path)
    ap.add_argument("original", type=Path)
    ap.add_argument("out", type=Path)
    ap.add_argument("--original2", type=Path)
    ap.add_argument("--ours2", type=Path)
    args = ap.parse_args()
    if not DIFF.exists():
        sys.exit(f"build it first: cargo build --release -p bb-render --example frame_diff ({DIFF})")
    args.out.mkdir(parents=True, exist_ok=True)
    lines = []
    for original in sorted(args.original.glob("*.png")):
        scene = original.stem
        ours = args.ours / original.name
        if not ours.exists():
            lines.append(f"{scene}: no frame of ours")
            continue
        cmd = [str(DIFF), str(ours), str(original), str(args.out / original.name)]
        for flag, other in (("--noise", args.original2), ("--noise-ours", args.ours2)):
            if other is not None and (other / original.name).exists():
                cmd += [flag, str(other / original.name)]
        run = subprocess.run(cmd, capture_output=True, text=True)
        if run.returncode != 0:
            lines.append(f"{scene}: {run.stderr.strip()}")
            continue
        report = run.stdout.splitlines()
        lines.append(f"{scene}: {report[0]}")
        lines += [f"  {r.strip()}" for r in report[1:7]]
    summary = "\n".join(lines)
    (args.out / "summary.txt").write_text(summary + "\n")
    print(summary)


if __name__ == "__main__":
    main()
