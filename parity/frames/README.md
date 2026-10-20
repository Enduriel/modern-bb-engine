# Frame parity

Whole frames of the original against ours: the same scene set up in both engines,
screenshotted in the original and drawn offscreen in ours, then diffed.

```
scenes.nut   the scenes (`probeScene(name)` on the world screen's script object) and
             the steps both engines walk (`::probeSteps`)
scenes.js    the original's side: walks the steps, shooting each scene once it settles
diff.py      diffs every scene, each engine's second run left out as noise
```

The steps: the intro event (then Huzzah!), the world map at dawn, a town, night on the
map, a battle (the UI goldens' pinned ambush) on the player's first turn, a short and a
long path, a skill aimed, effects (blood, a pool, a cloud), a zombie rising from the
ground at 40 and 80 %, units hidden by what is drawn in front of them (a unit behind
higher tiles, one sunk, one beside a tree, a boulder between two, then the camera cut
down to level 0), and the results screen and its loot. A step can poll a scene until it returns true (`until`) before its
shot, and click a page button after it. Ours (bb-app's `frame_scenes`) walks the same
list.

A probe with fewer steps before the battle gets a different battle map in each engine:
the map is drawn from `Math.rand` where `tactical_state.initMap` runs, and draws made
in between (the loading screen's tips, as the page answers) move it. Wrap
`::MapGen.get` to reseed first (for example: `local get = ::MapGen.get; ::MapGen.get <- function (t) { ::Math.seedRandom(4243); return get.call(::MapGen, t); }`).

## Running

```bash
# The original (one game launch; BB_PROBE_RUNNER is a probe runner that launches the
# original game, not part of this repository). The screen must be the game's configured
# video mode: a mode set at runtime doesn't take under the probe's gamescope. The probe
# logs the mode (`PROBE video mode`).
python3 "$BB_PROBE_RUNNER" parity/frames/scenes.nut \
    --js parity/frames/scenes.js --mode campaign --seed bbuitest \
    --size 2560x1440 --shots OUT/original --out OUT/original

# Ours, at the same mode
cd modern_bb_engine
cargo run --release -p bb-app --features ui --example frame_scenes -- OUT/ours --screen 2560x1440

# Every scene: per scene a heatmap and side-by-side crops of the largest regions, and
# OUT/diff/summary.txt. Run each engine more times (OUT/original2 OUT/original3 ...,
# OUT/ours2 ...) to leave out what differs between its own runs; give the original
# several (see below).
cargo build --release -p bb-render --example frame_diff
python3 ../parity/frames/diff.py OUT/ours OUT/original OUT/diff \
    --original2 OUT/original2 --ours2 OUT/ours2
```

`BbProbe.shot(name, callback)` in a page probe has the runner screenshot the
game's whole output (`gamescopectl screenshot` on the probe's own headless gamescope)
and calls back once the PNG is written, so a scene holds still until then.

## Reading a diff

- A pixel differs when a colour channel is off by more than the tolerance (24); a
  16-pixel cell when more than 5% of its pixels do. Text and edges never match exactly.
- Red in a heatmap differs, blue is noise: it differs between one engine's own runs too
  (2 pixels around it included). The runs only sample the noise: a random detail that
  every run of the original happens to draw the same way stays red (a brush flip agreed
  across four runs about 1 time in 8). The more runs, the fewer of those.
- The world's detail looks (which trees, their mirroring, ground scatter) come from an
  engine RNG the original seeds from the clock, so those regions differ between any two runs of the original too.
- Ours leaves out its drawn cursor: the original's screenshots have none. No mouse
  moves in either: the scenes park the scripts' cursor over the top-left panel, where
  hovering shows nothing (the original's tactical tooltips fail on a cursor never set).

## Regression check (ours against ours)

`bb-app/tests/frame_regression.rs` draws the same scenes and compares each with our
own frames from before, so a change to what the game draws fails the suite without any
probe of the original:

```bash
cd modern_bb_engine
cargo test --release -p bb-app --features ui --test frame_regression
```

- Run it in release, as the script takes the goldens: a debug build draws the battles
  0.1-0.4% differently (2026-10-19), beyond their allowance.
- The goldens are PNGs outside the repository, in the folder `BB_FRAME_GOLDENS` names.
  Without them the test skips and says so.
- After a change that is meant to show, take them again with
  `parity/frames/update_regression_goldens.sh`, and look at them first.
- The engine's own randomness (clouds, map details) is seeded, so two runs draw the
  same: the map and UI scenes identically, the battles within 0.031%.
- `regression.txt` holds how much of each scene may differ, and rectangles left out.
- A failing scene's heatmap and frame land in the test's build directory
  (`target/tmp/frame_regression/`).
