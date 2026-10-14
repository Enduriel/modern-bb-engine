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
long path, a skill aimed, effects (blood, a pool, a cloud), and the results screen
and its loot. A step can poll a scene until it returns true (`until`) before its
shot, and click a page button after it. Ours (bb-app's `frame_scenes`) walks the same
list.

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
# OUT/diff/summary.txt. Run each engine twice (OUT/original2, OUT/ours2) to leave out
# what differs between its own runs.
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
- Red in a heatmap differs, blue is noise: it differs between one engine's own two runs
  too (2 pixels around it included). Two runs only sample the noise: a tree that both
  of the original's runs happened to leave out stays red.
- The world's detail looks (which trees, their mirroring, ground scatter) come from an
  engine RNG the original seeds from the clock, so those regions differ between any two runs of the original too.
- Ours leaves out its drawn cursor: the original's screenshots have none. No mouse
  moves in either: the scenes park the scripts' cursor over the top-left panel, where
  hovering shows nothing (the original's tactical tooltips fail on a cursor never set).
