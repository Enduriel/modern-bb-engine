# Parity suite

Ground truth from the original game, kept as re-runnable checks. Each probe is a Squirrel
script that logs `PROBE <key> = <type> <value>` lines. `capture.py` runs it in the
original game and stores the output as a golden. `bb-engine/tests/parity.rs` runs the
same script in our engine and compares the two line by line.

```
probes/<group>/<name>.nut    the probes
goldens/<group>/<name>.txt   their output in the original
divergences.txt              known, accepted differences
capture.py, probes.py        capture from the original
```

## Running

```bash
cd modern_bb_engine
cargo test --release -p bb-engine --test parity              # compare (needs the Steam install)
PARITY_DUMP=/tmp/ours cargo test --release -p bb-engine --test parity   # also write our output

python3 parity/capture.py                  # recapture every golden from the original
python3 parity/capture.py menu/math        # only the launch that contains this probe
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

## Divergences

When we differ on purpose, or can't match (uninitialized memory, the C runtime's float
formatting), list the key in `divergences.txt` as `probe | key | reason`. A listed key
that starts matching again fails the test, so the list stays current.

Original behavior that is buggy or limiting, but that we match anyway, isn't listed here.
