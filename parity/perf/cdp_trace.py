#!/usr/bin/env python3
"""What Chromium does for the game's page, from a trace over DevTools.

    python3 parity/perf/cdp_trace.py PORT SECONDS [--frames-per-second N] [--out TRACE.json]
    python3 parity/perf/cdp_trace.py --read TRACE.json [--frames-per-second N]
    python3 parity/perf/cdp_trace.py PORT SECONDS --js-profile

`--js-profile` samples the page's JavaScript instead (the Profiler domain on the page
target, 100 µs sampling) and prints the functions by self and total time (ms a second).

PORT is the page's DevTools port (`bb --devtools PORT`). Records a Chromium trace of every
process (the browser side in the game's process, the renderer, any GPU or utility process)
for SECONDS and prints, per process and thread, the time its tasks ran (the `toplevel`
category's task events) and how many there were; for the renderer's main thread, the
time by kind of work (scripts by entry: timers, animation frames, events, XHR callbacks;
style, layout, paint, commit; garbage collection), the scripts' top functions, and the
XHRs (the bridge's round trips) a second. Times are inclusive (a function call's time
includes what it called). With `--frames-per-second` (the game's frame rate in that
window), also per game frame. `--out` keeps the raw trace (`chrome://tracing`,
Perfetto). The standard library only.
"""
import json, os, sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from cdp_memory import WS, http_json  # noqa: E402

CATEGORIES = [
    "toplevel", "devtools.timeline", "disabled-by-default-devtools.timeline", "v8", "v8.execute",
    "blink", "cc", "viz", "gpu", "ipc", "mojom", "benchmark", "disabled-by-default-v8.gc",
    "loading", "blink.user_timing", "input",
]

# The renderer main thread's work, by event name: the kind it's counted as.
KINDS = {
    "FunctionCall": "script: function call (entry)",
    "TimerFire": "script: timers",
    "FireAnimationFrame": "script: animation frames",
    "EventDispatch": "script: DOM events",
    "XHRReadyStateChange": "script: XHR callbacks",
    "XHRLoad": "script: XHR load",
    "EvaluateScript": "script: evaluate",
    "v8.compile": "script: compile",
    "v8.callFunction": "script: v8.callFunction",
    "RunMicrotasks": "script: microtasks",
    "UpdateLayoutTree": "style",
    "ScheduleStyleRecalculation": "style (scheduled)",
    "Layout": "layout",
    "PrePaint": "pre-paint",
    "Paint": "paint",
    "Layerize": "layerize",
    "Commit": "commit",
    "HitTest": "hit test",
    "MinorGC": "GC minor",
    "MajorGC": "GC major",
    "V8.GCScavenger": "GC scavenger",
    "V8.GCFinalizeMC": "GC mark-compact",
    "V8.GC_MC_INCREMENTAL": "GC incremental marking",
    "BlinkGC.AtomicPhase": "GC Blink (Oilpan)",
    "ParseHTML": "parse HTML",
    "ParseAuthorStyleSheet": "parse CSS",
    "Decode Image": "image decode",
    "ImageDecodeTask": "image decode",
    "BeginMainThreadFrame": "frame: begin main frame",
    "ThreadControllerImpl::RunTask": "tasks (all)",
}

TOP_TASKS = {"ThreadControllerImpl::RunTask", "ThreadPool_RunTask", "TaskGraphRunner::RunTask",
             "SequenceManager::DoWork", "RunTask", "ThreadControllerImpl::DoWork"}


def record(port, secs, out):
    import time
    version = http_json(port, "/json/version")
    ws = WS(version["webSocketDebuggerUrl"])
    events = []
    ws.call("Tracing.start", {"traceConfig": {"recordMode": "recordContinuously", "includedCategories": CATEGORIES},
                              "transferMode": "ReportEvents"}, events)
    time.sleep(secs)
    ws.send("Tracing.end")
    trace = []
    while True:
        m = ws.recv()
        if m.get("method") == "Tracing.dataCollected":
            trace += m["params"]["value"]
        elif m.get("method") == "Tracing.tracingComplete":
            break
    if out:
        with open(out, "w") as f:
            json.dump({"traceEvents": trace}, f)
    return trace


def complete(trace):
    """'X' events, and 'B'/'E' pairs made into 'X' events: (pid, tid, name, cat, ts, dur, args)."""
    out, open_ = [], defaultdict(list)
    for e in trace:
        ph = e.get("ph")
        if ph == "X" and "dur" in e:
            out.append((e["pid"], e["tid"], e["name"], e.get("cat", ""), e["ts"], e["dur"], e.get("args", {})))
        elif ph == "B":
            open_[(e["pid"], e["tid"])].append(e)
        elif ph == "E":
            stack = open_[(e["pid"], e["tid"])]
            if stack:
                b = stack.pop()
                out.append((b["pid"], b["tid"], b["name"], b.get("cat", ""), b["ts"], e["ts"] - b["ts"], b.get("args", {})))
    return out


def analyze(trace, fps):
    procs, threads = {}, {}
    for e in trace:
        if e.get("ph") == "M":
            if e["name"] == "process_name":
                procs[e["pid"]] = e["args"]["name"]
            elif e["name"] == "thread_name":
                threads[(e["pid"], e["tid"])] = e["args"]["name"]
    xs = complete(trace)
    if not xs:
        print("no events")
        return
    t0 = min(x[4] for x in xs)
    t1 = max(x[4] + x[5] for x in xs)
    secs = (t1 - t0) / 1e6
    per = lambda us: us / 1000 / secs  # ms a second

    def frame(us):
        return f", {us / 1000 / secs / fps:.3f} ms/frame" if fps else ""

    print(f"trace: {secs:.2f} s, {len(xs)} events")
    # Busy time per thread: top-level tasks, outermost only.
    busy, count = defaultdict(float), defaultdict(int)
    by_thread = defaultdict(list)
    for x in xs:
        if x[2] in TOP_TASKS:
            by_thread[(x[0], x[1])].append(x)
    for key, tasks in by_thread.items():
        tasks.sort(key=lambda x: x[4])
        end = -1
        for x in tasks:
            if x[4] >= end:
                busy[key] += x[5]
                count[key] += 1
                end = x[4] + x[5]
    print("\nper thread: task time (ms a second = % of a core x10), tasks a second")
    for (pid, tid), us in sorted(busy.items(), key=lambda kv: -kv[1]):
        if us / secs < 500:
            continue
        print(f"  {procs.get(pid, pid)}/{threads.get((pid, tid), tid)}: {per(us):.2f} ms/s ({per(us) / 10:.1f}% of a core){frame(us)}, "
              f"{count[(pid, tid)] / secs:.0f} tasks/s")
    # The renderer's main thread.
    mains = [k for k, n in threads.items() if n == "CrRendererMain"]
    for key in mains:
        print(f"\nrenderer main thread ({procs.get(key[0], key[0])}): work by kind, inclusive")
        kinds, kcount = defaultdict(float), defaultdict(int)
        funcs = defaultdict(float)
        fcount = defaultdict(int)
        xhr = 0
        for x in xs:
            if (x[0], x[1]) != key:
                continue
            kind = KINDS.get(x[2])
            if kind is None and x[2].startswith("V8.GC"):
                kind = "GC (other V8.GC*)"
            if kind:
                kinds[kind] += x[5]
                kcount[kind] += 1
            if x[2] == "FunctionCall":
                d = x[6].get("data", {})
                name = f"{d.get('functionName') or '(anonymous)'} {d.get('url', '').rsplit('/', 1)[-1]}:{d.get('lineNumber', '')}"
                funcs[name] += x[5]
                fcount[name] += 1
        for e in trace:
            if e.get("name") in ("ResourceSendRequest",) and (e.get("pid"), e.get("tid")) == key:
                xhr += 1
        for k, us in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {k}: {per(us):.2f} ms/s{frame(us)}, {kcount[k] / secs:.0f}/s")
        print(f"  requests sent (XHR, the bridge's round trips): {xhr / secs:.0f}/s")
        print("  top functions (entries from the event loop), inclusive:")
        for k, us in sorted(funcs.items(), key=lambda kv: -kv[1])[:15]:
            print(f"    {per(us):7.3f} ms/s{frame(us)}  {fcount[k] / secs:6.0f}/s  {k}")
    # Every thread's events by name (the largest), for the other processes' threads.
    print("\nlargest event kinds per process (inclusive ms a second, nesting double-counts):")
    names = defaultdict(float)
    for x in xs:
        names[(procs.get(x[0], x[0]), threads.get((x[0], x[1]), x[1]), x[2])] += x[5]
    for (p, t, n), us in sorted(names.items(), key=lambda kv: -kv[1])[:40]:
        print(f"  {per(us):8.2f} ms/s  {p}/{t}: {n}")


def js_profile(port, secs):
    import time
    page = next(t for t in http_json(port, "/json/list") if t.get("type") == "page")
    ws = WS(page["webSocketDebuggerUrl"])
    ws.call("Profiler.enable")
    ws.call("Profiler.setSamplingInterval", {"interval": 100})
    ws.call("Profiler.start")
    time.sleep(secs)
    profile = ws.call("Profiler.stop")["profile"]
    nodes = {n["id"]: n for n in profile["nodes"]}
    parent = {}
    for n in profile["nodes"]:
        for c in n.get("children", []):
            parent[c] = n["id"]
    deltas = profile["timeDeltas"]
    self_us, total_us = defaultdict(float), defaultdict(float)
    name = lambda n: f"{n['callFrame']['functionName'] or '(anonymous)'} {n['callFrame']['url'].rsplit('/', 1)[-1]}:{n['callFrame']['lineNumber'] + 1}"
    for sample, dt in zip(profile["samples"][1:], deltas[1:]):
        node = nodes[sample]
        self_us[name(node)] += dt
        seen, i = set(), sample
        while i is not None:
            k = name(nodes[i])
            if k not in seen:
                total_us[k] += dt
                seen.add(k)
            i = parent.get(i)
    span = (profile["endTime"] - profile["startTime"]) / 1e6
    print(f"JS profile over {span:.2f} s (ms a second)")
    print("  by self time:")
    for k, us in sorted(self_us.items(), key=lambda kv: -kv[1])[:25]:
        print(f"    {us / 1000 / span:8.3f}  {k}")
    print("  by total time:")
    for k, us in sorted(total_us.items(), key=lambda kv: -kv[1])[:25]:
        print(f"    {us / 1000 / span:8.3f}  {k}")


def main():
    args = sys.argv[1:]
    if "--js-profile" in args:
        js_profile(int(args[0]), float(args[1]))
        return
    fps = float(args[args.index("--frames-per-second") + 1]) if "--frames-per-second" in args else None
    if args[0] == "--read":
        trace = json.load(open(args[1]))
        trace = trace["traceEvents"] if isinstance(trace, dict) else trace
    else:
        out = args[args.index("--out") + 1] if "--out" in args else None
        trace = record(int(args[0]), float(args[1]), out)
    analyze(trace, fps)


if __name__ == "__main__":
    main()
