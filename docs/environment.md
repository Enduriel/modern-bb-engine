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
| `BB_NO_MODS=1` | Play vanilla (as `--no-mods`): the install's mods are mounted by default. |
| `BB_SAVES_DIR=DIR` | Keep the saves in `DIR` (`--saves` overrides it; empty is as if unset). By default `bb` keeps them in the original game's savegames folder (`Documents\Battle Brothers\savegames` on Windows; on Linux the same in the Proton prefix of the Steam install played, if the original has run there), else in `battle-brothers-modern/savegames` in the user's data folder; `saves.dir` in the settings file names another (see `bb-app/src/saves/mod.rs`). Only one `bb` at a time plays from a folder: a second one started on it says the game is already running and ends (`bb-app/src/saves/lock.rs`); `bb`s on different folders run side by side. The tests always keep theirs in a folder of their own. |
| `BB_LOGS_DIR=DIR` | Write `log.html`, `cef.log` and the earlier sessions' `logs/` to `DIR`, not to `battle-brothers-modern/` in the user's data folder (`--logs` does the same). |
| `BB_SCRIPT_CACHE=DIR` | Keep the compiled-script cache in `DIR` (default `$XDG_CACHE_HOME/bb-modern-engine/scripts`, else `~/.cache/…`); `off` or `0` turns the cache off. |
| `BB_SHEET_CACHE=DIR` | Keep the engine's own sprite sheets (each atlas's frames cut out, see `bb_assets::sheet_cache`) in `DIR`, not under the engine's cache folder; `off` keeps none. |
| `BB_AUDIO_BUFFER=FRAMES` | The sound device's buffer, in frames (default: the device's). |
| `BB_ERROR_DIALOG=0` | No message box when `bb` can't start (a machine that can't run the game, a game folder that won't load, any other error), stops on an error once the game runs, or crashes (`bb_app::failure`); the message still goes to stderr and `log.html`. The window tests set it. |

## Measuring

| Variable | What it does |
| --- | --- |
| `BB_STARTUP_TIMES=1` | Print the start's phases to stderr (`startup: <phase> <ms>`, milliseconds since the process started; `squirrel_bb::startup`). |
| `BB_STATS=1` | Log the game's total CPU and memory, its CEF processes included, every 5 s, with the frames left undrawn, those presented again undrawn as the mouse moved, and those the page rested through, and the page's paints (`page:`: how many, the time each took in the paint callback, its textures opened, its reads back). |
| `BB_FRAME_TIMES=PATH` | Write each frame's time by span (the step's parts, the draw's, the page's, the bridge's calls) to `PATH`, with the display's refresh as the frame clock takes it (`refresh`), with the page's paints since the last frame (`page_paints#`, `page_copy`: their time in the paint callback on the browser's thread, `page_imports#`, `page_reads#`, `page_read`) `re-presented#=1` on a frame that presented the last again as the mouse moved, and the mouse's path: `mouse_moves#` (the system's pointer moves since the last frame), `mouse_moves_delivered#` (those merged moves handed on: one a frame, or one before a button or the wheel), `input_page` (handing the page the pointer, a button, the wheel), `input_park` and `input_scripts` (parking the page and running the states' `onMouseInput`: a button or the wheel between frames; `input_scripts` also times the frame's merged move, in the step), `page_park` (the step waiting for the page to park: at most 50 ms a park, after which the frame goes on without its scripts until the page has parked) and `page_release` (a memory release after a discard, from sending it to its last part's answer, in the frame it finished; log.html has each one's parts as a debug line); see `parity/perf/frame_breakdown.py` and `page_capture.py`. A player on Windows can record it with `player_capture.py`'s `.bat`. |
| `BB_FRAME_TIMES_GPU=1` | With `BB_FRAME_TIMES`, each frame waits for the GPU, so its GPU time counts too. |
| `BB_GPU_TIMES=1` | With `BB_FRAME_TIMES`, each GPU submission's time from timestamp queries. |
| `BB_SCRIPT_PROFILE=REPORT.md` | Profile the scripts by function, file and mod, a report every `BB_SCRIPT_PROFILE_SECS` (10) seconds, or every `BB_SCRIPT_PROFILE_FRAMES` frames. |
| `BB_HEAP_PROFILE_DIR=DIR` | Under a preloaded jemalloc with profiling on (`LD_PRELOAD=libjemalloc.so.2 MALLOC_CONF=prof:true`, builds without `ui`; Linux and macOS), dump a heap profile into `DIR` every `BB_HEAP_PROFILE_SECS` seconds (10, or 10 again for a value that isn't a positive number). |
| `BB_TEXTURE_USAGE=1` | Track which texture pages the frames drew from; reported as `bb` closes. |
| `BB_UI_SLOW_CALLS=MS` | Log every call between the game and the page that takes at least `MS` milliseconds (`0`: all of them), to stderr. |
| `BB_PAGE_REST_LOG=1` | Log each change in whether the page rests between frames, and every 1,000 frames what gave it its turns. |

Without any of these, a page that keeps the game waiting is a warning in log.html (tag `UI`, and on stderr), with its time to the millisecond: a park answered a second or more after it was asked (`the page parked 1.402 s after it was asked to`), one not answered within 5 s, and a call into the page not answered within 5 s.
| `BB_ALLOW_TRACE=1` | Let any process of the user's trace `bb` (Linux's Yama), so a hung run's stacks can be read (`eu-stack`). The window tests set it. |

## Comparing engine choices

Each turns off, or changes, something the engine does by default, to measure what it
gives.

| Variable | What it does |
| --- | --- |
| `BB_PAGE_REST=0` | The page never rests between frames. `BB_PAGE_REST=1` lets it rest even while DevTools serve it (`--devtools` turns resting off otherwise). |
| `BB_PAGE_RELEASE=0` | The page gives no memory back where the game discards a scene (a battle's end, the campaign left, a save loaded) nor after the boot (`bb-ui/src/host/memory.rs`). A comma-separated list picks the release's parts instead, to compare them: `critical` or `moderate` (the memory-pressure signal the renderer gets), `gc` (a memory-reducing V8 collection), `trim` (the renderer's allocator gives its free memory back); the default is `moderate,gc,trim`. |
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
| `BB_ALLOW_SKIPS=1` | Let a test skip (saying `skipping: <why>`) when the machine lacks what it needs: the Steam install, a mod, a GPU, gamescope. Unset, such a test fails, and `scripts/test_all.py` fails a run with skips in it. |
| `BB_DATA_DIR=DIR` | The game's `data` folder (or the game's folder) `bb`, the tests and the examples use, ahead of the folder the player chose and Steam's install, as `--data` does for `bb` ([how bb finds the game](requirements.md#the-game-itself)). One without the game stops `bb` with that error. Default for the tests and examples: the game in this machine's Steam libraries (`bb_engine::install`). |
| `BB_STEAM_DIR=DIR` | Steam's folder: `bb` and the tests look for the game in its libraries only, instead of in the Steam folders `steamlocate` finds (a folder without Steam in it hides this machine's install: the window tests' not-found cases). The original's Proton prefix, for the probes and save fixtures, is under it too (default there `~/.local/share/Steam`). |
| `BB_TEST_FOLDER_DIALOG=DIR` | The not-found window's folder dialog answers `DIR` at once instead of opening (`bb_app::install::picker`). |
| `BB_ORIGINAL_SAVES=DIR` | Saves the original wrote, for the save-format tests; the tests fail without them (skip with `BB_ALLOW_SKIPS=1`). |
| `BB_ENGINE_SEED=N` | Seed the engine's own randomness (not the scripts' generator), for runs that must repeat exactly. |
| `BB_TEST_UNPRESENTED=N` | Draw frame `N` but don't present it (the window tests' stalled-compositor case). |
| `BB_TEST_SURFACE_FAILS=N` | The window's next `N` surface configures ask for an empty surface, which wgpu refuses (the window tests' case of a surface the system won't take, `bb_app::surface_trouble`). |
| `BB_TEST_PRESENT_EVERY=K/N` (or `N`, as `2/N`) | Draw and present the first `K` of every `N` frames whatever they show (the window tests' frame-pacing cases). |
| `BB_TEST_MOUSE_MOVES=A-B` | Frames `A` to `B` (not included) each start with the mouse moved, a pixel or so near the window's top left corner (the window tests' stand-in for a player's mouse). |
| `BB_GAME_QUEUE_TURN=PATH` | A turn in the shared game-display queue that the runner (`scripts/test_all.py`) already holds, so `window.rs` waits once. |
| `BB_UI_DISPLAY`, `BB_UI_DISPLAY_STATUS` | Set by the tests' display wrapper: where a wrapped process runs (a headless gamescope by default) and where it writes its exit status. |
| `BB_ICON_TEST=1` | Lets the (ignored) tests that read the game's icon back from a window run (`bb_app::icon`), with `--include-ignored`; only inside a display of its own: a headless gamescope's X server for the X11 one, a headless KWin for the Wayland one (`BB_ICON_WAYLAND=none`: a compositor without the protocol, as gamescope's; `BB_ICON_HOLD=SECONDS` keeps the window up). |
| `BB_CLIPBOARD_TEST=1` | Lets the (ignored) tests that copy to and paste from a real clipboard run, with `--include-ignored`; only inside a display of its own (a headless gamescope, or a headless KWin for the Wayland one). |
| `BB_SCENE_KEY_MODS=1` | Run `bb-engine`'s scene-key test with the install's mods. |
| `BB_PACKAGE_RPATH` | See [packaging](packaging.md). |
| `BB_LLVM_MINGW=DIR` | An unpacked llvm-mingw for `scripts/windows_cross.py` (the Windows cross-build), instead of the pinned release it fetches into the target folder. |

Build-time variables, set by the build scripts rather than by hand: `BB_CEF_DIR` (where
the CEF distribution was found), `BB_STEAM_TARGET` (the target the pinned Steam library
is chosen for) and `BB_PACKAGE_HOST` (the target `bb-package` packages for by default).

`bb-render`'s `battle_still` example also reads `GROUND=1` (time the ground alone: the
tiles without their details, nothing on them) and `SHOW_POS=1` (print each entity that
moved between frames).
