# Parity suite

Ground truth from the original game, kept as re-runnable checks. Each probe is a Squirrel
script that logs `PROBE <key> = <type> <value>` lines. `capture.py` runs it in the
original game and stores the output as a golden. `bb-engine/tests/parity.rs` runs the
same script in our engine and compares the two line by line.

```
probes/<group>/<name>.nut    the probes
ui_payloads/<name>.nut, .js  what the scripts send the UI along a walk of the game
goldens/<group>/<name>.txt   their output in the original
divergences.txt              known, accepted differences
missing_natives.txt          natives the original binds that we don't have yet
capture.py, probes.py        capture from the original
perf/                        performance baselines, see below
```

## Running

```bash
cargo test --release -p bb-engine --test parity              # compare (needs the Steam install)
cargo test --release -p bb-engine --test ui_payloads         # the UI payloads
PARITY_DUMP=/tmp/ours cargo test --release -p bb-engine --test parity   # also write our output

python3 parity/capture.py                  # recapture every golden from the original
python3 parity/capture.py menu/math        # only the launch that contains this probe
python3 parity/capture.py ui_payloads/world
```

Capturing needs a probe runner that launches the original game (not part of this
repository; point `BB_PROBE_RUNNER` at it). Probes are grouped by mode and seed, and each group is one game launch,
so add probes to an existing group when you can.

## Writing a probe

The first line says how the game is started:

```squirrel
// mode: menu
// mode: campaign seed=rust-engine-test
```

`menu` runs at the main menu. `campaign` starts a new campaign with that seed (first
player banner, default settings) and runs once the world is up and paused.

Every probe of a group is concatenated into one script, in name order. Each one is
wrapped in its own function and `try`, behind a `PROBE_SECTION <name>` marker, so one
failing probe can't hide the others. A probe still sees the world as the probes before it
in the group left it: don't change the world in a campaign probe, or put that probe last.

Keep the output comparable:
- No `<` or `>` in values. The original's `log.html` mangles them.
- No raw ids. World tile `ID`s are memory addresses and entity ids come from a global
  counter: print relative values, or check a property (unique, ascending).
- Whitespace runs collapse to one space on both sides.
- Code runs with single-precision float math, as game scripts do in the original.
- Some code crashes the original instead of raising an error, which loses the whole
  launch. Compiling `base++` does (ours reports a compile error).

A probe that has to watch the game over time sets `::BbProbe.DeferDone = true` and
calls `::BbProbe.done()` from a scheduled event, as with the probe harness; the
runner here then runs frames until it does. Only a group's last probe can defer.

A probe with a `// compare: subset` line is a coverage check: only the original's lines
must appear in ours, and our extras are fine. `menu/natives` and `campaign/natives` list
every native function reachable from each global (through delegate chains and class
bases) and on the objects the world hands out, so a native the original binds and we
lack fails by name. `campaign/natives_objects` does the same for every other kind of
native object the scripts get hold of, in the world and in a battle (it defers), and
lists their properties too (the keys of a class's `__getTable` and `__setTable`).

## UI payloads

The scripts' side of the UI goldens (bb-ui keeps the DOM goldens).
`ui_payloads/world.nut` replaces every UI handle with a stand-in that records each
`asyncCall`/`call` payload, then walks the steps of bb-ui's world goldens: the intro
event, its button, the character screen, the nearest town, leaving it, and a bandit
ambush (pinned as bb-ui's battle golden pins it) up to the player's first turn. In the
original, `world.js` paces the steps 3 s apart from the UI; `tests/ui_payloads.rs` runs
3 s of frames between them. Each distinct payload is logged once per step, one line per leaf,
with tables in sorted key order. Values that can't match between runs are normalized in
the recorder, the same way in both engines: numeric ids (and the bare ids functions in
`__payloadIdArgs` take) become first-seen ordinals, and portraits' content ids (from the
unseeded C runtime `rand()`) become `rand`.

## Performance baselines

```bash
python3 parity/perf/perf.py                  # the original: writes perf/original.txt
cargo run --release -p bb-engine --example perf_baseline    # ours, next to it
```

`perf.py` takes two launches. In menu mode it records the time to the menu, the memory
there and the script benches (`perf/benches.nut`, timed with the game's live clock
`Time.getExactTime`). In campaign mode it records the new campaign's time, the world
benches (`perf/benches_world.nut`), the frame rate on the running map, and save/load
times. Memory is the game process's resident set, sampled from outside while the run
holds the game. The save is `bb_probe_perf` and is deleted afterwards.

They aren't all like for like: the original's memory includes Wine, the renderer and the
UI, its saves go to disk (ours stay in memory), and its frame rate has no headless
counterpart yet.

## Divergences

When we differ on purpose, or can't match (uninitialized memory, the C runtime's float
formatting), list the key in `divergences.txt` as `probe | key | reason`. Natives we
haven't implemented yet go in `missing_natives.txt`, in the same format: it's the
worklist the coverage probes produce. A key with one `*` matches every key with that
prefix and suffix. A listed key that starts matching again, or a `*` entry that excuses
nothing, fails the test, so both lists stay current.

Original behavior that is buggy or limiting, but that we match anyway, isn't listed here.
