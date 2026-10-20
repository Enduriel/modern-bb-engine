#!/usr/bin/env python3
"""The game's processes over an interval, from /proc (Linux): CPU and wake-ups per
thread, memory by kind per process, GPU memory per process.

    python3 parity/perf/proc_sample.py SECONDS [--pid PID] [--json OUT.json] [--memory]

PID is `bb` (default: the newest process named `bb`); every process it started (CEF's)
is included. CPU is a share of one core (100% = a whole core). Wake-ups are context
switches a second (voluntary: the thread blocked and was woken; involuntary: preempted),
each thread's `voluntary_ctxt_switches` + `nonvoluntary_ctxt_switches`. With
`--memory`, each process's PSS by the kind of mapping holding it (the same kinds as
`bb_app::stats`: C heap, anonymous, V8, PartitionAlloc, libcef.so, GPU driver, shared,
stacks, files) and the GPU memory its DRM clients allocated (fdinfo: VRAM, GTT), read at
the end. The standard library only.
"""
import glob, json, os, sys, time

HZ = os.sysconf("SC_CLK_TCK")


def children(pid):
    out = []
    for t in glob.glob(f"/proc/{pid}/task/*/children"):
        try:
            out += [int(x) for x in open(t).read().split()]
        except OSError:
            pass
    return out


def tree(root):
    todo, seen = [root], []
    while todo:
        p = todo.pop()
        seen.append(p)
        todo += children(p)
    return seen


def newest(name):
    best = None
    for d in os.listdir("/proc"):
        if d.isdigit():
            try:
                # The game's own process, not CEF's (the same binary, run with --type=).
                if open(f"/proc/{d}/comm").read().strip() == name and role(int(d)) == "game":
                    best = max(best or 0, int(d))
            except OSError:
                pass
    return best


def role(pid):
    try:
        args = open(f"/proc/{pid}/cmdline").read().split("\0")
    except OSError:
        return "?"
    kind = next((a[7:] for a in args if a.startswith("--type=")), None)
    if kind is None:
        return "game"
    sub = next((a.split("=", 1)[1] for a in args if a.startswith("--utility-sub-type=")), None)
    return f"{kind}:{sub.rsplit('.', 1)[-1]}" if sub else kind


def threads(pids):
    """(pid, tid, name) -> (cpu ticks, context switches)."""
    s = {}
    for p in pids:
        for t in glob.glob(f"/proc/{p}/task/*"):
            try:
                st = open(t + "/stat").read()
                name = st[st.index("(") + 1 : st.rindex(")")]
                f = st[st.rindex(")") + 2 :].split()
                switches = 0
                for line in open(t + "/status"):
                    if line.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")):
                        switches += int(line.split()[1])
                s[(p, int(t.rsplit("/", 1)[1]), name)] = (int(f[11]) + int(f[12]), switches)
            except (OSError, ValueError):
                pass
    return s


DRIVERS = ["/dev/dri", "libvulkan_", "libgallium", "libLLVM", "libdrm", "libnvidia", "libGLX_", "libEGL_", "amdgpu"]


def kind(name):
    if name == "[heap]":
        return "C heap"
    if name == "":
        return "anonymous"
    if name.startswith("[stack"):
        return "stacks"
    if name.startswith("[anon:v8"):
        return "V8 heap"
    if name.startswith("[anon:partition_alloc"):
        return "PartitionAlloc"
    if name.startswith("["):
        return "anonymous"
    if "libcef.so" in name:
        return "libcef.so"
    if any(d in name for d in DRIVERS):
        return "GPU driver"
    if name.startswith(("/memfd:", "/dev/shm", "/SYSV")):
        return "shared memory"
    return "other files"


def memory(pid):
    kinds, current = {}, "anonymous"
    try:
        for line in open(f"/proc/{pid}/smaps"):
            if line.startswith("Pss:"):
                kinds[current] = kinds.get(current, 0) + int(line.split()[1]) * 1024
            else:
                head = line.split(None, 1)[0]
                if "-" in head and all(c in "0123456789abcdef-" for c in head):
                    parts = line.split(None, 5)
                    current = kind(parts[5].strip() if len(parts) > 5 else "")
    except OSError:
        pass
    return kinds


def gpu(pid):
    clients, vram, gtt = set(), 0, 0
    for fd in glob.glob(f"/proc/{pid}/fdinfo/*"):
        try:
            info = open(fd).read()
        except OSError:
            continue
        fields = {}
        for line in info.splitlines():
            if ":" in line:
                k, v = line.split(":", 1)
                fields[k.strip()] = v.strip()
        client = fields.get("drm-client-id")
        if client is None or client in clients:
            continue
        clients.add(client)

        def size(key):
            v = fields.get(key)
            if not v:
                return 0
            n, *unit = v.split()
            return int(n) << {"KiB": 10, "MiB": 20, "GiB": 30}.get(unit[0] if unit else "", 0)

        vram += max(size("drm-memory-vram"), size("drm-total-local0"))
        gtt += max(size("drm-memory-gtt"), size("drm-total-system0"))
    return vram, gtt


def sample(root, secs, with_memory):
    pids = tree(root)
    a = threads(pids)
    t0 = time.time()
    time.sleep(secs)
    b = threads(pids)
    dt = time.time() - t0
    rows = []
    for k, (ticks, sw) in b.items():
        ticks0, sw0 = a.get(k, (ticks, sw))
        rows.append({"pid": k[0], "tid": k[1], "thread": k[2], "role": role(k[0]),
                     "cpu": (ticks - ticks0) / HZ / dt * 100, "wakeups": (sw - sw0) / dt})
    procs = []
    for p in pids:
        mine = [r for r in rows if r["pid"] == p]
        entry = {"pid": p, "role": role(p), "cpu": sum(r["cpu"] for r in mine),
                 "wakeups": sum(r["wakeups"] for r in mine), "threads": len(mine)}
        if with_memory:
            entry["pss"] = memory(p)
            entry["vram"], entry["gtt"] = gpu(p)
        procs.append(entry)
    return {"seconds": dt, "processes": procs, "threads": rows}


def report(s):
    mb = lambda b: b / 1048576
    total_cpu = sum(p["cpu"] for p in s["processes"])
    total_wake = sum(p["wakeups"] for p in s["processes"])
    print(f"over {s['seconds']:.1f} s: CPU {total_cpu:.1f}% of a core, {total_wake:.0f} wake-ups/s")
    for p in s["processes"]:
        line = f"  {p['role']} (pid {p['pid']}, {p['threads']} threads): CPU {p['cpu']:.1f}%, {p['wakeups']:.0f} wake-ups/s"
        if "pss" in p:
            line += f", PSS {mb(sum(p['pss'].values())):.0f} MB, VRAM {mb(p['vram']):.0f} MiB, GTT {mb(p['gtt']):.0f} MiB"
        print(line)
        if "pss" in p:
            parts = sorted(p["pss"].items(), key=lambda kv: -kv[1])
            print("    " + ", ".join(f"{k} {mb(v):.1f}" for k, v in parts if v >= 1 << 20))
    print("  busiest threads (CPU %, wake-ups/s):")
    for r in sorted(s["threads"], key=lambda r: -r["cpu"])[:20]:
        if r["cpu"] >= 0.1 or r["wakeups"] >= 50:
            print(f"    {r['cpu']:5.1f}% {r['wakeups']:6.0f}/s  {r['role']}:{r['thread']} ({r['tid']})")
    print("  most wake-ups:")
    for r in sorted(s["threads"], key=lambda r: -r["wakeups"])[:10]:
        if r["wakeups"] >= 1:
            print(f"    {r['wakeups']:6.0f}/s {r['cpu']:5.1f}%  {r['role']}:{r['thread']} ({r['tid']})")


def main():
    args = sys.argv[1:]
    secs = float(args[0]) if args and not args[0].startswith("--") else 5.0
    pid = int(args[args.index("--pid") + 1]) if "--pid" in args else newest("bb")
    if pid is None:
        sys.exit("no process named bb")
    s = sample(pid, secs, "--memory" in args)
    if "--json" in args:
        with open(args[args.index("--json") + 1], "w") as f:
            json.dump(s, f)
    report(s)


if __name__ == "__main__":
    main()
