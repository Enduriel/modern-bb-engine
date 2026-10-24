#!/usr/bin/env python3
"""Summarize `bb`'s per-frame times (`BB_FRAME_TIMES=PATH`) by phase and span.

    python3 parity/perf/frame_times.py FRAMES.txt [PHASES.json] [--budget MS] [--min-share PCT]

FRAMES.txt has a line a frame: `N interval=.. stepped=.. work=.. zoom=.. quads=.. refresh=.. t=.. span=ms ...
counter#=n ...` (see bb-app's `FrameTimes`). PHASES.json, if given, is a list of
`[name, first frame, last frame]`; otherwise the whole file is one phase. For each phase:
the frames, how many were drawn (presented) and how many left undrawn, the frame interval
and own work (p50 / p99 / mean), and every span and counter as mean, p50 and p99 per frame
(over all of the phase's frames, 0 where a frame didn't run it) and the mean's share of
the budget (default 5.556 ms, a 180 Hz frame). "Own work" counts the wait for a swapchain
image (`acquire`, where VSync waits) and, with `BB_FRAME_TIMES_GPU`/`BB_GPU_TIMES`, the
wait for the GPU (`gpu`); the row "less acquire" leaves both out. Spans nest;
shares of nested spans add up inside their parent, not to 100%.
"""
import json, sys


def parse(path):
    frames = []
    for line in open(path):
        parts = line.split()
        if not parts or not parts[0].isdigit():
            continue
        f = {"frame": int(parts[0]), "spans": {}, "counts": {}}
        for p in parts[1:]:
            k, _, v = p.partition("=")
            if k.endswith("#"):
                f["counts"][k[:-1]] = int(v)
            elif k in ("interval", "stepped", "work", "zoom", "quads", "refresh", "t"):
                f[k] = float(v)
            else:
                f["spans"][k] = f["spans"].get(k, 0.0) + float(v)
        frames.append(f)
    return frames


def pct(values, q):
    if not values:
        return 0.0
    v = sorted(values)
    return v[min(len(v) - 1, int(q * (len(v) - 1) + 0.5))]


def summarize(name, frames, budget, min_share):
    out = []
    if not frames:
        return f"### {name}\n\nno frames\n"
    drawn = [f for f in frames if "present" in f["spans"]]
    n = len(frames)
    secs = sum(f["interval"] for f in frames) / 1000
    out.append(f"### {name}\n")
    out.append(f"{n} frames over {secs:.1f} s ({n / secs if secs else 0:.0f}/s), {len(drawn)} drawn, {n - len(drawn)} undrawn; "
               f"budget {budget:.3f} ms\n")
    rows = [("frame interval", [f["interval"] for f in frames]), ("own work (all frames)", [f["work"] for f in frames]),
            ("own work less acquire (the wait for an image)", [f["work"] - f["spans"].get("acquire", 0.0) - f["spans"].get("gpu", 0.0) for f in frames])]
    if drawn and len(drawn) < n:
        rows.append(("own work (drawn frames)", [f["work"] for f in drawn]))
        rows.append(("own work (undrawn frames)", [f["work"] for f in frames if "present" not in f["spans"]]))
    names = []
    for f in frames:
        for k in f["spans"]:
            if k not in names:
                names.append(k)
    out.append("| per frame, ms | mean | p50 | p99 | max | % of budget (mean) |")
    out.append("|---|---:|---:|---:|---:|---:|")
    for label, vals in rows:
        mean = sum(vals) / len(vals) if vals else 0
        out.append(f"| {label} | {mean:.3f} | {pct(vals, .5):.3f} | {pct(vals, .99):.3f} | {max(vals) if vals else 0:.3f} | {mean / budget * 100:.1f}% |")
    for k in names:
        vals = [f["spans"].get(k, 0.0) for f in frames]
        mean = sum(vals) / n
        if mean / budget * 100 < min_share:
            continue
        out.append(f"| {k.replace('_', ' ')} | {mean:.4f} | {pct(vals, .5):.4f} | {pct(vals, .99):.4f} | {max(vals):.3f} | {mean / budget * 100:.2f}% |")
    counts = []
    for f in frames:
        for k in f["counts"]:
            if k not in counts:
                counts.append(k)
    if counts:
        out.append("\n| counter, per frame | mean | p50 | p99 | max |")
        out.append("|---|---:|---:|---:|---:|")
        for k in counts:
            vals = [f["counts"].get(k, 0) for f in frames]
            out.append(f"| {k.replace('_', ' ')} | {sum(vals) / n:.2f} | {pct(vals, .5):.0f} | {pct(vals, .99):.0f} | {max(vals)} |")
    return "\n".join(out) + "\n"


def main():
    args = sys.argv[1:]
    budget = float(args[args.index("--budget") + 1]) if "--budget" in args else 1000 / 180
    min_share = float(args[args.index("--min-share") + 1]) if "--min-share" in args else 0.0
    positional = [a for i, a in enumerate(args) if not a.startswith("--") and (i == 0 or not args[i - 1].startswith("--"))]
    frames = parse(positional[0])
    phases = json.load(open(positional[1])) if len(positional) > 1 else [["all", 0, 1 << 62]]
    for name, first, last in phases:
        print(summarize(name, [f for f in frames if first <= f["frame"] <= last], budget, min_share))


if __name__ == "__main__":
    main()
