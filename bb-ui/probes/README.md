# UI bridge probes

Probes for the ORIGINAL game, run with a probe runner that launches the original game
(not part of this repository; point `BB_PROBE_RUNNER` at it). They established the
Coherent bridge's behavior that `bb-ui` reproduces, and captured the UI parity
goldens.

| Probe | Checks |
|---|---|
| `bridge_order.nut` + `.js` | **Inline calls**: `asyncCall`, `call` and `UI.connect` run their JS before returning, and `SQ.call` runs its script before the next JS statement. Also: `callSQ` return values and timing, void calls never resolving, handles, the native `engine` members, `compatMode`, dynamic script order, `coui://` resolution. |
| `bridge_values.nut` + `.js` | **Conversions**: JS→SQ numbers, holes, `undefined`, functions; SQ→JS non-string keys, closures, instances, weakrefs. Also `RootScreen`'s handles, return values, and JS exceptions. |
| `font_metrics.js` | **Normal line heights** of the game's fonts at every whole size from 6 to 40px (block and inline heights). The original's answers are `tests/goldens/font_metrics.json`. |
| `over_ui.nut` + `.js` | **`UI.setCursorPosition`'s "over the UI" answer**: full-height test stripes, asked at their centres in page pixels and at half those coordinates. Partly settled. |
| `ui_scale.nut` + `.js` | **UI scale**: the page's viewport, `devicePixelRatio` and zoom before and after the scripts set `UIScale` 1.5, and a snapshot of the scaled main menu (prepend `src/parity/snapshot.js`). The result is `tests/goldens/main_menu_ui150.json`. It changes the video mode, so it relies on the runner restoring `config.cfg`. |
| `layout_rules.js` | **Old-WebKit layout rules**: the 8px font minimum, and which boxes count toward `scrollWidth`/`scrollHeight`. The results are `tests/goldens/layout_rules.json` (the `fonts` answer is short, so the runner prints it inline). |
| `world_golden.js` | **World screen golden**: campaign mode (`--mode campaign --seed bbuitest`), snapshotted once the origin's intro event shows. Prepend `src/parity/snapshot.js` as for `ui_goldens.js`. The result is `tests/goldens/world_intro.json`; our side starts the campaign the same way (`host::goldens::capture_world`). |
| `ui_goldens.js` | **UI parity goldens**: snapshots of the main menu, the new-campaign page and Options, for `tests/goldens/`. Run it with `src/parity/snapshot.js` prepended (`cat snapshot.js ui_goldens.js > probe.js`) and a one-line `.nut` probe. The `js_*_golden_*.json` results are the goldens. |

Run from the repo root (menu mode, hidden and muted; about a minute each), with
`BB_PROBE_RUNNER` naming the probe runner:

```bash
python3 "$BB_PROBE_RUNNER" bb-ui/probes/bridge_order.nut \
    --js bb-ui/probes/bridge_order.js --mode menu --out <out-dir>/bridge_order
```

**Don't** add an SQ array holding a closure or instance to `asyncCall` data: the original
crashes ("Array size mismatch").
