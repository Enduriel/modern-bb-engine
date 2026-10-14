# Frame parity

Whole frames of the original against ours: the same scene set up in both engines,
screenshotted in the original and drawn offscreen in ours, then diffed.

```
scenes.nut   the scenes: `probeScene(name)` on the world screen's script object
scenes.js    the original's side: closes the intro event, sets each scene up, shoots it
```

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
cargo run --release -p bb-app --features ui --example frame_scenes -- OUT/ours --screen 2560x1440 world

# The diff: a heatmap, the differing regions, and side-by-side crops of the largest
cargo run --release -p bb-render --example frame_diff -- OUT/ours/world.png OUT/original/world.png OUT/world-heat.png
```

`BbProbe.shot(name, callback)` in a page probe has the runner screenshot the
game's whole output (`gamescopectl screenshot` on the probe's own headless gamescope)
and calls back once the PNG is written, so a scene holds still until then.

## Reading a diff

- A pixel differs when a colour channel is off by more than the tolerance (24); a
  16-pixel cell when more than 5% of its pixels do. Text and edges never match exactly.
- The world's detail looks (which trees, their mirroring, ground scatter) come from an
  engine RNG the original seeds from the clock, so those regions differ between any two runs of the original too.
- Ours leaves out its drawn cursor: the original's screenshots have none.
