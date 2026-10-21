# Environment variables

Every environment variable `bb`, its tests and its tools read. Players need none of
them: `bb`'s command-line options (the notes at the top of `bb-app/src/main.rs`) and
its settings file cover play. A variable that takes
`0` to turn something off is on whenever it's set to anything else, unless it says
otherwise.

## Playing

| Variable | What it does |
| --- | --- |
| `BB_NO_STEAM=1` | Never load Steam's library (as `--no-steam`): achievements and stats stay off. |
| `BB_STEAM_API=PATH` | Load Steam's library from `PATH` (as `--steam-api`). |
| `BB_SAVES_DIR=DIR` | Keep the saves in `DIR`, not in `battle-brothers-modern/savegames` in the user's data folder. |
| `BB_LOGS_DIR=DIR` | Write `log.html`, `cef.log` and the earlier sessions' `logs/` to `DIR`, not to `battle-brothers-modern/` in the user's data folder (`--logs` does the same). |
| `BB_SCRIPT_CACHE=DIR` | Keep the compiled-script cache in `DIR` (default `$XDG_CACHE_HOME/bb-modern-engine/scripts`, else `~/.cache/…`); `off` or `0` turns the cache off. |
| `BB_SHEET_CACHE=DIR` | Keep the engine's own sprite sheets (each atlas's frames cut out, see `bb_assets::sheet_cache`) in `DIR`, not under the engine's cache folder; `off` keeps none. |
| `BB_AUDIO_BUFFER=FRAMES` | The sound device's buffer, in frames (default: the device's). |

## Measuring

| Variable | What it does |
| --- | --- |
| `BB_STATS=1` | Log the game's total CPU and memory, its CEF processes included, every 5 s, with the frames left undrawn and those the page rested through. |
| `BB_FRAME_TIMES=PATH` | Write each frame's time by span (the step's parts, the draw's, the page's, the bridge's calls) to `PATH`; see `parity/perf/frame_breakdown.py`. |
| `BB_FRAME_TIMES_GPU=1` | With `BB_FRAME_TIMES`, each frame waits for the GPU, so its GPU time counts too. |
| `BB_GPU_TIMES=1` | With `BB_FRAME_TIMES`, each GPU submission's time from timestamp queries. |
| `BB_SCRIPT_PROFILE=REPORT.md` | Profile the scripts by function, file and mod, a report every `BB_SCRIPT_PROFILE_SECS` (10) seconds, or every `BB_SCRIPT_PROFILE_FRAMES` frames. |
| `BB_HEAP_PROFILE_DIR=DIR` | Under a preloaded jemalloc with profiling on (`LD_PRELOAD=libjemalloc.so.2 MALLOC_CONF=prof:true`, builds without `ui`; Linux and macOS), dump a heap profile into `DIR` every `BB_HEAP_PROFILE_SECS` seconds (10, or 10 again for a value that isn't a positive number). |
| `BB_TEXTURE_USAGE=1` | Track which texture pages the frames drew from; reported as `bb` closes. |
| `BB_UI_SLOW_CALLS=MS` | Log every call between the game and the page that takes at least `MS` milliseconds (`0`: all of them), to stderr. |
| `BB_PAGE_REST_LOG=1` | Log each change in whether the page rests between frames, and every 1,000 frames what gave it its turns. |
| `BB_ALLOW_TRACE=1` | Let any process of the user's trace `bb` (Linux's Yama), so a hung run's stacks can be read (`eu-stack`). The window tests set it. |

## Comparing engine choices

Each turns off, or changes, something the engine does by default, to measure what it
gives.

| Variable | What it does |
| --- | --- |
| `BB_PAGE_REST=0` | The page never rests between frames. `BB_PAGE_REST=1` lets it rest even while DevTools serve it (`--devtools` turns resting off otherwise). |
| `BB_BOOT_RELEASE=0` | Keep what the page's boot left behind, instead of giving it back once the boot is over (about -45 MB at the main menu). |
| `BB_TRIM=0` | Don't hand the allocator's free pages back to the system behind loading screens (glibc's `malloc_trim`, Linux). |
| `BB_TEXTURE_PAGES=off` (or `0`) | Draw from the game's whole atlases instead of the engine's own pages of the frames drawn (`bb_render::pages`). |
| `BB_LAZY_LEVELS=0` | Make the script classes' levels as plain clones, not lazy ones. |
| `BB_LAZY_MEMBERS=0` | Copy an instance's member tables (`m`) in full, not lazily. |
| `BB_UI_CPU_RASTER=1` | Rasterize the page on the CPU (`--disable-gpu-rasterization`). |
| `BB_UI_V8_FLAGS=FLAGS` | V8's flags for the page, instead of `--optimize-for-size`. |
| `BB_UI_SWITCHES="name=value name2"` | More Chromium switches for the page (features join the engine's lists; V8 flags as `js-flags=--flag`). |
| `BB_UI_OZONE=x11` or `wayland` | Which display protocol the page's browser uses on Linux (default: Wayland where `WAYLAND_DISPLAY` is set, else X11). |

## Tests and tools

| Variable | What it does |
| --- | --- |
| `BB_DATA_DIR=DIR` | The game's `data` folder the tests and examples use (default: the developer's Linux Steam install). |
| `BB_STEAM_DIR=DIR` | Steam's folder (default `~/.local/share/Steam`). |
| `BB_ORIGINAL_SAVES=DIR` | Saves the original wrote, for the save-format tests; the tests skip without them. |
| `BB_ENGINE_SEED=N` | Seed the engine's own randomness (not the scripts' generator), for runs that must repeat exactly. |
| `BB_TEST_UNPRESENTED=N` | Draw frame `N` but don't present it (the window tests' stalled-compositor case). |
| `BB_TEST_PRESENT_EVERY=K/N` (or `N`, as `2/N`) | Draw and present the first `K` of every `N` frames whatever they show (the window tests' frame-pacing cases). |
| `BB_GAME_QUEUE_TURN=PATH` | A turn in the shared game-display queue that the runner (`scripts/test_all.py`) already holds, so `window.rs` waits once. |
| `BB_UI_DISPLAY`, `BB_UI_DISPLAY_STATUS` | Set by the tests' display wrapper: where a wrapped process runs (a headless gamescope by default) and where it writes its exit status. |
| `BB_CLIPBOARD_TEST=1` | Run the test that copies to and pastes from the display's real clipboard; only inside a display of its own (a headless gamescope). |
| `BB_SCENE_KEY_MODS=1` | Run `bb-engine`'s scene-key test with the install's mods. |
| `BB_PACKAGE_RPATH` | See [packaging](packaging.md). |

Build-time variables, set by the build scripts rather than by hand: `BB_CEF_DIR` (where
the CEF distribution was found), `BB_STEAM_TARGET` (the target the pinned Steam library
is chosen for) and `BB_PACKAGE_HOST` (the target `bb-package` packages for by default).

`bb-render`'s `battle_still` example also reads `GROUND=1` (time the ground alone: the
tiles without their details, nothing on them) and `SHOW_POS=1` (print each entity that
moved between frames).
