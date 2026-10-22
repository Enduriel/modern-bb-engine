#!/usr/bin/env python3
"""Summarize a capture's frames by phase: how often the game stepped, presented and the page
painted, and what the page's paint callback cost. For a player's capture
(`player_capture.py`'s `.bat`) or a `frame_breakdown.py` run.

    python3 parity/perf/page_capture.py DIR

DIR has `frames.txt` (`BB_FRAME_TIMES`) and the game's output with its `PHASE name t=..` lines
(`output.txt` from the `.bat`, `stderr.txt` from `frame_breakdown.py`). A phase runs from its
line to the next; its first second is left out (the page settling after a change). Per phase:
steps a second (what bb's FPS display shows), frames presented a second (what the player
sees), page paints a second, the paint callback's time per paint (mean, p99, max: importing
the paint's texture, copying it and waiting for the GPU, on the browser's thread), and the
frame interval's p99. Then `frame_times.py`'s whole summary, its phases from the same lines.
"""
import json, os, re, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import frame_times  # noqa: E402

PHASE = re.compile(r"PHASE (.+?) t=([\d.]+)")
SETTLE = 1.0


def phases(directory):
    for name in ("output.txt", "stderr.txt"):
        path = os.path.join(directory, name)
        if os.path.exists(path):
            text = open(path, errors="replace").read()
            return [(m.group(1), float(m.group(2))) for m in PHASE.finditer(text)]
    sys.exit(f"no output.txt or stderr.txt in {directory}")


def pct(values, q):
    values = sorted(values)
    return values[min(len(values) - 1, int(q * len(values)))] if values else 0.0


def main():
    directory = sys.argv[1]
    frames = frame_times.parse(os.path.join(directory, "frames.txt"))
    marks = phases(directory)
    ranges = []
    print("| phase | s | steps/s | presented/s | paints/s | callback ms mean | p99 | max | interval p99 ms |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for (name, start), (_, end) in zip(marks, marks[1:] + [("", float("inf"))]):
        if name == "end":
            continue
        inside = [f for f in frames if start + SETTLE <= f.get("t", -1) < end]
        if len(inside) < 2:
            continue
        ranges.append([name, inside[0]["frame"], inside[-1]["frame"]])
        secs = inside[-1]["t"] - inside[0]["t"]
        presented = sum(1 for f in inside if f["spans"].get("present") is not None)
        paints = [f["counts"].get("page_paints", 0) for f in inside]
        callback = [f["spans"]["page_copy"] / f["counts"]["page_paints"] for f in inside if f["counts"].get("page_paints")]
        intervals = [f["interval"] for f in inside]
        mean = sum(callback) / len(callback) if callback else 0.0
        print(f"| {name} | {secs:.1f} | {len(inside) / secs:.0f} | {presented / secs:.0f} | {sum(paints) / secs:.0f} | "
              f"{mean:.2f} | {pct(callback, 0.99):.2f} | {max(callback, default=0.0):.2f} | {pct(intervals, 0.99):.2f} |")
    path = os.path.join(directory, "page_capture_phases.json")
    with open(path, "w") as f:
        json.dump(ranges, f)
    print()
    sys.stdout.flush()
    subprocess.run([sys.executable, os.path.join(HERE, "frame_times.py"), os.path.join(directory, "frames.txt"), path,
                    "--min-share", "0.05"])


if __name__ == "__main__":
    main()
