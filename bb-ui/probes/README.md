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
| `text_bytes.nut` + `.js` | **Text bytes**: which UTF-16 code units the page gets for script strings built byte by byte (UTF-8, Windows-1252, invalid, with a NUL), and which bytes the scripts get for characters from the page. One string per call, each logged first: one the original can't convert ends it with "string too long". |
| `ui_scale.nut` + `.js` | **UI scale**: the page's viewport, `devicePixelRatio` and zoom before and after the scripts set a `UIScale` (`scale` in the `.nut`, `SCALE_NAME` in the `.js`: 1.5 and `ui150` as written), and a snapshot of the scaled main menu (prepend `src/parity/snapshot.js`). The results are `tests/goldens/main_menu_ui150.json` and, at 1.25, `main_menu_ui125.json`. It changes the video mode, so it relies on the runner restoring `config.cfg`. |
| `layout_rules.js` | **Old-WebKit layout rules**: the 8px font minimum, and which boxes count toward `scrollWidth`/`scrollHeight`. The results are `tests/goldens/layout_rules.json` (the `fonts` answer is short, so the runner prints it inline). |
| `text_rules.js` | **Old-WebKit text layout**: whole-pixel glyph advances, and no kerning. Measures 3 s after placing its samples (the original loads a web font once text uses it). The results are `tests/goldens/text_rules.json`. |
| `border_rules.js` | **Border images on a box with no border style**: the border widths stay, in layout too. Declared in a style sheet, measured a second later. The results are `tests/goldens/border_rules.json`. |
| `modded_goldens.js` | **UI goldens with the install's mods** (menu mode, `--keep-mods`, `src/parity/snapshot.js` prepended): the main menu once MSU's "Mod Options" shows, and MSU's mod settings screen (left by Cancel, which saves nothing). The results are `tests/goldens/modded_main_menu.json` and `modded_mod_options.json`. Retaken 2026-10-20 after updates to MSU (1.8.0 → 1.9.0), Dev Console, Modular Vanilla and Reforged Core. Retaken 2026-10-23 after updates to Swifter (1.3.1), EIMO (10.0.7), Clever Recruiter (2.1.7), Configurable Pause (1.0.7) and World Parameters (1.1.0): what changed are the stored MSU settings (Dev Console's debug keybinds now off, so its debug info box is hidden; Clever Recruiter's pre-tryout options) and the probe's cursor (none now, so Dev Console's element inspector box stays on the mod settings screen; see `mouse_events.js`). |
| (not yet) | **Modded campaign goldens.** `world_golden.{nut,js}` with `--keep-mods` doesn't work in the original (2026-10-18): the probe runner's campaign start runs twice (two `PROBE_CAMPAIGN_START`), and each time MSU's `settings_screen.nut:26` `show` fails ("does not have a JSHandle"), so no intro event comes. Modded campaign probes need a probe runner fix first. MSU 1.9.0 doesn't change that path (`settings_screen.nut`'s `show` and its late `connect_screens.js` are as in 1.8.0; not rerun). |
| `loading_race.nut` | **The loading screen shown again while it fades out** (menu mode, SQ only): each trial shows it, hides it, shows it 0–1500 ms after the hide, and logs whether it ended shown. Within the fade (≤ 400 ms) the show is lost in the original. |
| `font_families.js` | **The generic font families**: the width of one sample in `monospace`, `serif`, `sans-serif`, Windows' fonts and a missing one, and `monospace`'s line. The results are `tests/goldens/font_families.json`. |
| `hit_opacity.js` | **Hit testing through invisible boxes** (`--keep-mods` for Modern Hooks' popup): which element a point hits under `opacity: 0`, `0.01` and `visibility: hidden`. |
| `hooks_resume.js` | **Modern Hooks' resume** (`--keep-mods`): `Hooks.screenInit` and its "not loading" panel once the menu is up. Taken with MSU 1.8.0, when Modern Hooks had an error to show. |
| `mouse_events.js` | **The probe's cursor** (`--keep-mods`): the mouse events the page gets with no input sent, and when Dev Console's element inspector box (hidden by any mouseover) appears and hides. `tests/mods_ui.rs` puts our cursor where the original's was. |
| `callback_rules.nut` + `.js` | **Page calls' callbacks**: whether the callback gets the scripts' answer when the script returns plainly, calls into the page, connects a screen, or creates one. |
| `centring_rules.js` | **Centring by `auto` margins** with half a pixel of room left over, or less room than the box: where old WebKit's whole pixels put the box. Declared in a style sheet, measured a second later. The results are `tests/goldens/centring_rules.json`. |
| `background_tiles.js` | **Tiled backgrounds** (menu mode, `--shots DIR --size 2560x1440`): a screenshot of the main menu, then a card of 36 boxes whose background image repeats or not, covers its box or not, and their rects. Which boxes the original draws upside down (read off the card's screenshot) is `tests/goldens/background_tiles.json`. |
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
