#!/usr/bin/env python3
"""What holds our engine's C heap, by owner: a jemalloc heap profile read,
unsampled as jeprof does it, each stack symbolized from the binaries' symbol tables (release
builds have no debug info) and charged to its first frame outside the allocator and the
standard containers.

The profiles come from `memory_inventory`, which dumps one at each point of its session
when it runs under jemalloc with profiling on. Built without the UI: CEF doesn't run under a
preloaded jemalloc (it crashes at start), so the page's processes aren't in these.

    cd modern_bb_engine
    cargo build --release -p bb-app --example memory_inventory
    mkdir -p /path/to/profiles
    LD_PRELOAD=/usr/lib/x86_64-linux-gnu/libjemalloc.so.2 MALLOC_CONF=prof:true,lg_prof_sample:16 \\
      BB_HEAP_PROFILE_DIR=/path/to/profiles target/release/examples/memory_inventory [--battles N]
    python3 parity/perf/heap_owners.py /path/to/profiles/*.heap            # by owner
    python3 parity/perf/heap_owners.py --detail 40 one.heap                  # the top functions

With several profiles, the owners are columns, one per profile, in the order given.
"""

import bisect
import math
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

# The allocator, the standard library and containers, and the archives' file reads: a stack
# is charged to the first frame past these.
SKIP = re.compile(
    r"^(\?\?|malloc|calloc|realloc|free|posix_memalign|aligned_alloc|je_|_rjem|imalloc|__rust_|__rdl|"
    r"alloc::|core::|std::|hashbrown::|<alloc::|<core::|<std::|<hashbrown::|operator new|"
    r"sq_vm_malloc|sq_vm_realloc|<T as |<\[T\]|<Vec|<\[|__libc|_dl_|smallvec|<smallvec|indexmap::|"
    r"<indexmap|arrayvec|<&|<str |memchr|bytemuck|<u8|SQ_MALLOC|sqvector<|"
    # A file read from the archives is its reader's: the sound bank's, the renderer's.
    r"<bb_engine::vfs::DatArchive>::read|<bb_engine::files::GameFiles>::read)"
)

# Owners, by the charged frame's symbol: the first that matches.
OWNERS = [
    ("Squirrel VM: tables", r"^SQTable::|^sq_newtable|^RefTable::"),
    ("Squirrel VM: closures and environments", r"^sq_bindenv|^SQClosure::|^SQNativeClosure::|^sq_newclosure|^SQVM::CLOSURE_OP"),
    ("Squirrel VM: compiled functions", r"^SQFunctionProto::|^SQCompiler|^SQFuncState::|^sq_compile|^SQLexer"),
    ("Squirrel VM: strings", r"^SQStringTable::|^SQString::|^sq_pushstring"),
    ("Squirrel VM: class instances", r"^SQClass::|^SQInstance::|^sq_newclass|^sq_createinstance|^sq_newuserdata|^SQUserData"),
    ("Squirrel VM: arrays", r"^SQArray::|^sq_newarray|^sq_arrayappend|^sq_arrayresize"),
    ("Squirrel VM: other", r"^SQ|^sq_|^SQVM::"),
    ("squirrel crate (bindings)", r"^<*squirrel::"),
    ("squirrel-bb: class system", r"^<*squirrel_bb::(class|module|state)::"),
    ("squirrel-bb: natives", r"^<*squirrel_bb::native::"),
    ("squirrel-bb: other", r"^<*squirrel_bb::"),
    ("bb-engine: archives (zip indexes)", r"^<*zip::|^<?bb_engine::(vfs|files|datsource)"),
    ("bb-engine: other", r"^<*bb_engine::"),
    ("bb-assets", r"^<*bb_assets::"),
    ("bb-render", r"^<*bb_render::"),
    ("bb-audio and sound decoding", r"^<*(bb_audio|kira|symphonia|cpal)"),
    ("bb-ui", r"^<*bb_ui::"),
    ("bb-app", r"^<*bb_app::"),
    ("wgpu, naga", r"^<*(wgpu|naga|gpu_allocator|ash)"),
    ("GPU driver", r"@@lib(gallium|vulkan|drm)|^vk_|^radv|^amdgpu|^ac_|^aco"),
    ("window system", r"^<*(winit|wayland|xkb|smithay|calloop|sctk)|^wl_|^xkb_"),
]
OWNERS = [(name, re.compile(pattern)) for name, pattern in OWNERS]


def read_profile(path):
    """(stacks as (addresses, bytes), mappings as (start, end, path), load bases)."""
    text = Path(path).read_text()
    head, _, maps = text.partition("MAPPED_LIBRARIES:")
    period = int(re.match(r"heap_v2/(\d+)", head).group(1))
    stacks, cur = [], None
    for line in head.splitlines():
        if line.startswith("@"):
            cur = [int(a, 16) for a in line[1:].split()]
        elif cur is not None and line.strip().startswith("t*:"):
            count, size = map(int, re.match(r"\s*t\*:\s*(\d+):\s*(\d+)", line).groups())
            if count:
                # jeprof's unsampling: a sample stands for 1 / (1 - e^(-size/period)).
                stacks.append((cur, size / (1 - math.exp(-(size / count) / period))))
            cur = None
    modules, base = [], {}
    for line in maps.splitlines():
        parts = line.split()
        if len(parts) >= 6 and parts[5].startswith("/"):
            start, end = (int(x, 16) for x in parts[0].split("-"))
            if int(parts[2], 16) == 0:
                base.setdefault(parts[5], start)  # ELF addresses are relative to it
            if "x" in parts[1]:
                modules.append((start, end, parts[5]))
    return stacks, modules, base


_symbols = {}


def symbols(path):
    """A module's function symbols, sorted by address, from nm."""
    if path not in _symbols:
        rows = []
        for flags in (["-C", "--defined-only"], ["-C", "-D", "--defined-only"]):
            out = subprocess.run(["nm", *flags, path], capture_output=True, text=True).stdout
            for line in out.splitlines():
                parts = line.split(" ", 2)
                if len(parts) == 3 and parts[0] and parts[1] in "TtWwi":
                    rows.append((int(parts[0], 16), parts[2]))
        rows.sort()
        _symbols[path] = ([a for a, _ in rows], [n for _, n in rows])
    return _symbols[path]


def symbolize(addr, modules, base):
    for start, end, path in modules:
        if start <= addr - 1 < end:  # a return address points past its call
            addrs, names = symbols(path)
            i = bisect.bisect_right(addrs, addr - 1 - base.get(path, start)) - 1
            return names[i] if i >= 0 else "?? " + os.path.basename(path)
    return "??"


def charged(stack, modules, base):
    frames = [symbolize(a, modules, base) for a in stack]
    return next((f for f in frames if not SKIP.match(f)), frames[0] if frames else "??")


def owner(frame):
    return next((name for name, pattern in OWNERS if pattern.search(frame)), "other")


def main():
    args = sys.argv[1:]
    detail = 0
    if args and args[0] == "--detail":
        detail, args = int(args[1]), args[2:]
    if not args:
        sys.exit(__doc__)
    columns = []
    for path in args:
        stacks, modules, base = read_profile(path)
        by_owner, by_frame = defaultdict(float), defaultdict(float)
        for stack, size in stacks:
            frame = charged(stack, modules, base)
            by_owner[owner(frame)] += size
            by_frame[frame] += size
        columns.append((Path(path).stem, by_owner, by_frame))
    mb = lambda b: f"{b / 2**20:.1f}"
    owners = sorted({o for _, c, _ in columns for o in c}, key=lambda o: -max(c.get(o, 0) for _, c, _ in columns))
    print("| owner | " + " | ".join(name for name, _, _ in columns) + " |")
    print("|---|" + "---:|" * len(columns))
    print("| **live heap** | " + " | ".join(f"**{mb(sum(c.values()))}**" for _, c, _ in columns) + " |")
    for o in owners:
        print(f"| {o} | " + " | ".join(mb(c.get(o, 0)) for _, c, _ in columns) + " |")
    if detail:
        name, _, frames = columns[-1]
        print(f"\nThe largest functions in {name}:\n")
        for frame, size in sorted(frames.items(), key=lambda x: -x[1])[:detail]:
            print(f"  {mb(size):>8} MB  {frame[:150]}")


main()
