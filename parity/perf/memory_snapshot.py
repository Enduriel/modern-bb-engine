#!/usr/bin/env python3
"""Read bb's memory snapshots (`bb_app::memory_snapshot`: Ctrl+Shift+F11 or SIGUSR1 in a
running game writes `memory-<time>/` in its log folder):

    python3 parity/perf/memory_snapshot.py SNAPSHOT_DIR              # one, by category
    python3 parity/perf/memory_snapshot.py OLD_DIR NEW_DIR [--all]   # what grew between two

Each category's figures come from the folder's `snapshot.json`: the processes' PSS by
role and kind of memory (and the GPU memory they allocated), the C allocator, wgpu's
allocator and the renderer's textures, the engine's services, the Squirrel VM's census by
object kind, and Chromium's own figures per process and allocator. The comparison lists
the figures that changed by at least 1 MiB (or 100 objects), largest change first (all of
them with `--all`). Processes are matched by role (their pids differ between runs), and
Chromium's by name. The standard library only.
"""

import json
import sys
from collections import defaultdict
from pathlib import Path

MIB = 1 << 20


def load(path):
    p = Path(path)
    if p.is_dir():
        p = p / "snapshot.json"
    with open(p, encoding="utf-8") as f:
        return json.load(f)


def figures(snap):
    """Every figure as {(category, name): (value, unit)}, unit "B" (bytes) or "n" (a count)."""
    out = {}

    def put(cat, name, value, unit="B"):
        if isinstance(value, (int, float)):
            out[(cat, name)] = (out.get((cat, name), (0, unit))[0] + value, unit)

    procs = (snap.get("processes") or {}).get("processes", [])
    for p in procs:
        role = p.get("role", "?")
        put("processes", f"{role}: PSS", p.get("pss", 0))
        for kind, b in (p.get("pss_by_kind") or {}).items():
            put("processes", f"{role}: PSS {kind}", b)
        put("processes", f"{role}: VRAM", p.get("vram", 0))
        put("processes", f"{role}: GTT", p.get("gtt", 0))
        for name, b in (p.get("figures") or {}).items():
            if name in ("VmRSS", "RssAnon", "RssFile", "RssShmem", "VmSwap", "WorkingSetSize", "PrivateUsage", "phys_footprint"):
                put("processes", f"{role}: {name}", b)
    put("processes", "all: PSS", sum(p.get("pss", 0) for p in procs))
    put("processes", "all: GPU (VRAM+GTT)", sum(p.get("vram", 0) + p.get("gtt", 0) for p in procs))

    a = snap.get("allocator") or {}
    for key in ("in_use", "free_held", "mmapped_bytes", "releasable_top"):
        put("allocator", key, a.get(key))
    arenas = a.get("arenas") or []
    if arenas:
        put("allocator", "arenas", len(arenas), "n")
        put("allocator", "arenas: from the system", sum(x["system"] for x in arenas))
        put("allocator", "arenas: free in them", sum(x["free_fast"] + x["free_rest"] for x in arenas))

    g = snap.get("gpu") or {}
    textures = g.get("textures") or []
    put("gpu", "renderer textures", sum(t["bytes"] for t in textures))
    put("gpu", "renderer textures (count)", len(textures), "n")
    put("gpu", "renderer pages", sum(p["bytes"] for p in g.get("pages") or []))
    put("gpu", "renderer pages: frames", sum(p["frames"] for p in g.get("pages") or []), "n")
    put("gpu", "renderer pages: frames' texels", sum(p.get("used_texels", 0) for p in g.get("pages") or []), "n")
    put("gpu", "renderer pages: frames loading", g.get("frames_loading"), "n")
    put("gpu", "glyph pages", (g.get("glyph_pages") or {}).get("bytes"))
    for key in ("frame_target", "ui_page", "fog_mask", "own"):
        put("gpu", key, g.get(key))
    for c in g.get("drm_clients") or []:
        for key, v in (c.get("figures") or {}).items():
            put("gpu", f"game's DRM client: {key}", v)
    w = g.get("wgpu_allocator") or {}
    put("gpu", "wgpu: allocated", w.get("allocated"))
    put("gpu", "wgpu: reserved in blocks", w.get("reserved"))
    for row in w.get("by_name") or []:
        put("gpu", f"wgpu: {row['name'] or '(unnamed)'}", row["bytes"])

    for item in (snap.get("engine") or {}).get("items", []):
        put("engine", f"{item['owner']}: {item['what']}", item["bytes"])
        put("engine", f"{item['owner']}: {item['what']} (count)", item["count"], "n")

    for k in (snap.get("census") or {}).get("kinds", []):
        put("census", k["kind"], k["bytes"])
        put("census", f"{k['kind']} (count)", k["count"], "n")

    chromium = snap.get("chromium") or {}
    for key, v in (chromium.get("page") or {}).items():
        put("chromium", f"page: {key}", v, "n" if key.startswith("dom.") else "B")
    for p in chromium.get("processes", []):
        name = p.get("name") or f"pid {p.get('pid')}"
        for key, b in (p.get("totals") or {}).items():
            if isinstance(b, (int, float)) and not key.startswith("is_"):
                put("chromium", f"{name}: {key}", b)
        for alloc, b in (p.get("allocators") or {}).items():
            last = alloc.rsplit("/", 1)[-1]
            hashed = len(last) >= 15 and all(c in "0123456789abcdefABCDEF" for c in last)
            if alloc.count("/") <= 1 and not hashed:
                put("chromium", f"{name}: {alloc}", b)
        attrs = p.get("attributes") or {}
        # Live against kept: Oilpan's committed pages and live objects, malloc's mapping.
        for path, key in (("blink_gc/main/heap", "committed_size"), ("blink_gc/main/heap", "allocated_objects_size"),
                          ("malloc", "virtual_size"), ("partition_alloc/allocated_objects", None), ("malloc/allocated_objects", None)):
            if key is None:
                put("chromium", f"{name}: {path}", (p.get("allocators") or {}).get(path))
            else:
                put("chromium", f"{name}: {path} {key}", (attrs.get(path) or {}).get(key))
    return out


def fmt(value, unit):
    return f"{value / MIB:10.1f} MiB" if unit == "B" else f"{value:10.0f}    "


def summarize(snap):
    print(f"Memory snapshot {snap.get('time')} ({snap.get('version')}), asked by {snap.get('trigger')} at frame {snap.get('frame')}, "
          f"{'in a battle' if snap.get('in_battle') else 'not in a battle'}, written in {snap.get('ms', 0):.0f} ms")
    for part in snap.get("parts", []):
        state = "ok" if part.get("ok") else "FAILED: " + "; ".join(part.get("errors", []))
        print(f"  {part['name']:<10} {part.get('ms', 0):8.1f} ms  {state}")
    by_cat = defaultdict(list)
    for (cat, name), (value, unit) in figures(snap).items():
        by_cat[cat].append((name, value, unit))
    for cat, rows in by_cat.items():
        print(f"\n{cat}:")
        rows.sort(key=lambda r: (r[2] != "B", -r[1]))
        for name, value, unit in rows:
            if value:
                print(f"  {fmt(value, unit)}  {name}")


def compare(old, new, show_all):
    a, b = figures(old), figures(new)
    print(f"From {old.get('time')} (frame {old.get('frame')}) to {new.get('time')} (frame {new.get('frame')})")
    by_cat = defaultdict(list)
    for key in sorted(set(a) | set(b)):
        va, unit = a.get(key, (0, b.get(key, (0, "B"))[1]))
        vb, _ = b.get(key, (0, unit))
        d = vb - va
        big = abs(d) >= (MIB if unit == "B" else 100)
        if d and (show_all or big):
            by_cat[key[0]].append((key[1], va, vb, d, unit))
    if not by_cat:
        print("Nothing changed by 1 MiB or 100 objects.")
    for cat, rows in by_cat.items():
        print(f"\n{cat}:")
        rows.sort(key=lambda r: (r[4] != "B", -abs(r[3])))
        for name, va, vb, d, unit in rows:
            sign = "+" if d > 0 else "-"
            delta = f"{sign}{abs(d) / MIB:.1f} MiB" if unit == "B" else f"{sign}{abs(d):.0f}"
            print(f"  {delta:>14}  {fmt(va, unit).strip():>14} -> {fmt(vb, unit).strip():<14}  {name}")


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) == 1:
        summarize(load(args[0]))
    elif len(args) == 2:
        compare(load(args[0]), load(args[1]), "--all" in sys.argv)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
