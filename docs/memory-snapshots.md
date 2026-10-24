# Memory snapshots

When the game seems to use too much memory, or more the longer it runs, a memory snapshot
records where the memory is at that moment, in files that can be sent along with a report
and compared with a later snapshot. Taking one doesn't stop the game: it pauses for a
moment (well under a second, plus a second or two in which the UI's part comes in) and
plays on.

## Taking one

- **The hotkey: Ctrl+Shift+F11**, on every system. The game itself never sees this key
  (vanilla uses F11 in developer mode).
- **On Linux and macOS, the signal `SIGUSR1`**, sent to the game's process, without
  touching the game's window or input:

  ```sh
  kill -USR1 <pid>
  ```

  The game's process is the `bb` whose command line has no `--type=` (the others are its
  browser's processes, which ignore the signal), for example
  `pgrep -x bb | while read p; do grep -qa -- --type= /proc/$p/cmdline || echo $p; done`.

Nothing is gathered until a snapshot is asked for. A snapshot asked for while the last
is still being written is skipped (log.html says so).

## Where it goes

Into a new folder `memory-<time>/` (the time in UTC, e.g.
`memory-2026-10-24T09-30-12.345Z`) in the game's log folder, beside `log.html`: by
default `battle-brothers-modern/` in the user's data folder (`~/.local/share` on Linux,
`%APPDATA%` on Windows, `~/Library/Application Support` on macOS), or the folder `--logs`
names. log.html says when one starts and where it was written, and lists any part that
failed.

## What's in it

| File | What it holds |
| --- | --- |
| `summary.txt` | Everything below, in words. Start here. |
| `snapshot.json` | The same as data, for tools. |
| `census.txt` | Every object of the scripts' virtual machine (Squirrel) with its size: what `parity/perf/script_memory.py` reads. |
| `malloc-info.xml` | Linux: the C allocator's own report, arena by arena. |
| `chromium-trace.json` | With the UI: Chromium's own breakdown of its processes (memory-infra), as a trace that Perfetto (ui.perfetto.dev) or `chrome://tracing` opens. |
| `v8.heapsnapshot` | Only with `--memory-snapshot-v8`: V8's heap snapshot of the UI page, which Chrome's DevTools (Memory tab, Load) opens. Often 100 MB and more. |

The parts, in `summary.txt` and `snapshot.json`:

- **processes**: the game's process and each of its browser's, with the memory each
  takes (on Linux PSS and RSS, split into the C heap, other anonymous memory, V8's heap,
  Chromium's PartitionAlloc, libcef, GPU driver, shared memory, stacks and files, plus
  the GPU memory each allocated; on Windows each process's working set and private
  bytes; on macOS each one's footprint and resident size);
- **allocator**: what the C allocator hands out, and what it holds free without giving it
  back (Linux: per arena; macOS: its zones' totals; not yet on Windows);
- **gpu**: the renderer's textures (each with its size and when it was last drawn),
  its pages of frames (how full each is, and which scenes drew its frames), the frames
  still loading, its glyph pages, the frame and the UI page's texture, wgpu's allocator
  (what it allocated, in how much reserved memory, by name), and on Linux the game's DRM
  client as the driver counts it. On amdgpu the page's GL context (Chromium's GPU work,
  in the game's process) shares that client, so its VRAM is ours plus Chromium's (about
  150–190 MiB at 1080p: two driver screens, the 3 paint buffers, the page's pictures and
  tiles, and buffers radeonsi keeps after they're freed) plus RADV's own (about 36 MiB):
  compare it with wgpu's figures, not alone;
- **engine**: what the engine's services hold, counted and estimated (the world's
  entities and map, a battle's units and map, the sprite registry, the class system, the
  UI bridge, the sound bank, saves kept in memory);
- **census**: the scripts' objects by kind (tables, closures, strings, …);
- **chromium** (with the UI): Chromium's own figures per process and allocator, after a
  garbage collection, with what each heap holds live against what it keeps: Blink's
  Oilpan heap's committed pages against its live objects, malloc's and PartitionAlloc's
  objects in use against what they have from the system; and the page's own: V8's
  JavaScript heap (used of total, and the Blink objects it holds) and the DOM's node,
  document and listener counts. Growing live figures mean data kept alive; growing
  committed or retained figures with flat live ones mean memory kept after it was freed.

**Privacy:** `census.txt` holds the scripts' strings (names, texts of the campaign), and
`v8.heapsnapshot` the UI page's text and data. Look at them before sharing them publicly.

## Reading them

```sh
python3 parity/perf/memory_snapshot.py memory-2026-10-24T09-30-12.345Z               # one, by category
python3 parity/perf/memory_snapshot.py memory-<earlier> memory-<later> [--all]        # what grew
python3 parity/perf/script_memory.py memory-<…>/census.txt                            # the scripts' objects
```

The comparison lists every figure that changed by 1 MiB or more (or 100 objects),
largest change first: a snapshot early in a session and one after an hour of play show
what grew.

## Notes for developers

- The code: `bb-app/src/memory_snapshot/` (the trigger, the parts, the files),
  `squirrel_bb::memory_report` (the services' `ReportMemory`), `bb_ui::host::memory_dump`
  (Chromium's dump through CEF's own DevTools channel: no `--devtools` port needed),
  `bb_app::stats` (the processes, per system).
- The census is always built in (`SQVM::write_census`); the `census` feature only adds
  the census's access tracking (which tables the scripts read and write over a stretch
  of play), which hooks every table access and stays a measuring build's.
- The first snapshot with the UI starts Chromium's tracing service, a utility process of
  its own (about 55 MB PSS), which Chromium keeps until the game ends: the next
  snapshots list it, marked as the snapshot's.
- Chromium's dump is given up on after 20 s (180 s with the heap snapshot); what came by
  then is kept, and the part is marked failed.
- `BB_TEST_MEMORY_SNAPSHOT_KEY=N` presses the hotkey at frame N (the window tests'
  stand-in for a keyboard).
