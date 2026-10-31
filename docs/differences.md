# How bb differs from the original game

`bb` plays your own copy of Battle Brothers: the game's scripts, data, UI and mods run on
it unchanged. By default it does what the original does, quirks included, so mods that
rely on the original's behaviour keep working. Everything below is a deliberate
difference: a crash or limit of the original's engine that `bb` fixes, something new it
can do, or a trade-off made on purpose. Gameplay design is never changed.

The "Can mods notice?" column says whether a script or the UI's JavaScript could tell
the two engines apart. Mods that want to use what `bb` adds can check for it: see
[modding](modding.md).

## Fixes and improvements

### Bugs and crashes in the original's engine

Where the original crashes or corrupts its state, `bb` does something sensible instead.
No mod can rely on the original's behaviour in these cases.

| What | The original | bb | Can mods notice? |
| --- | --- | --- | --- |
| More than 64 factions in a battle | Faction ids from 64 up corrupt each tile's zone of control and visibility: odd AI and movement | Any number of factions works | Yes: mods with many factions get working battles |
| Zone-of-control highlight with 32+ factions | Faction ids from 32 up are mixed up with lower ones | Every faction is itself | No: only the drawn highlight |
| A sound file that doesn't exist | A critical error, and the game stops running | Logged, and nothing plays | Only broken mods |
| `Music.isPlaying()` before any music has started | Crashes | Returns false | Only mods that ask that early |
| `Time.clearEvents()` called from a scheduled event | Crashes (also a load started from one) | Clears every pending event and goes on | Only mods that crash the original |
| `Tactical.getTileSquare` off the battle map | Crashes; vanilla's road builder can reach it (rarely) | Gives the nearest tile on the map and logs a warning | No |
| Reading a world tile's `Properties`, a brush's `ID` | Crashes | Raises a script error ("does not exist") | Only mods that crash the original |
| A brush object used after its sprite is gone | Crashes | Still holds its `Name` and `TextureName` | Only mods that crash the original |
| Reading from a save stream that is being written (or the reverse) | Returns garbage; `beginRead` hangs the game | Raises a script error | Only broken mods |
| `self()` / `base()` on a script object | `base()` crashes, and so does using what `self()` returns | The object itself / its parent class level | Only mods that crash the original |
| A script containing `base++` | Crashes while compiling | A compile error | Only broken mods |
| A script array holding a function or instance sent to the UI | Crashes | Arrives in JavaScript as `null` | Only mods that crash the original |
| A broken UTF-16 character sent from the UI | Fails ("string too long") | Arrives as the replacement character | Only mods that fail in the original |
| A UI call into the scripts that calls back into the UI | Extra empty replies, often to older calls, can resolve the call with nothing before its real reply. With some mods (MSU) a battle's first turn logs "Failed to query entity data" and the turn bar stays locked | Each call gets exactly one reply, its own | No: mods that hit it get a working turn bar |
| A `logInfo` line longer than the log's buffer | The line is lost and a "Buffer overflow" error logged | Any length is written | No |

### New capabilities

| What | The original | bb | Can mods notice? |
| --- | --- | --- | --- |
| World map size | At most 140 × 170; a mod asking for more crashes worldgen | Any size the scripts ask for, also changed at the main menu without a restart; vanilla's size stays seed-exact | Yes: a mod setting the size gets that map |
| Fog of war on large maps | A fixed 2048² mask, coarser on a bigger map | The mask grows with the map | No: looks only |
| Text on the map in any language | Bitmap fonts with 256 Windows-1252 characters: UTF-8 shows as `Ã©`, other scripts as garbage | UTF-8 text is decoded, and characters the bitmap fonts lack are drawn from outline fonts; Windows-1252 text is drawn exactly as before | Yes: [`registerFont`](modding.md#fonts-for-text-in-any-language-registerfont-version-1) adds fonts for the map and the UI |
| Operating systems | Windows only | Windows, Linux and macOS ([requirements](requirements.md)) | No |
| Steam | Exits if Steam isn't running or doesn't own the game | Runs without Steam; achievements and stats work on a Steam install | Only through `isSteamBuild()` (below) |
| Preload lists (`preload/*.txt`) | Read at start; a mod must replace the whole file, so two such mods conflict | Never read: textures and sounds load when first used, without pop-in | Only as timing |
| Window and video mode | Size, fullscreen and borderless apply after a restart; the window can't be resized | Every change applies at once; a window can be resized (down to 1024 × 768) and the game follows it | Yes: `getVideoMode()` reports the new size at once |
| An FPS display | Vanilla's FPS box exists but is never shown | Shown with `bb --fps`, `ui.show_fps = true` in the settings file, or Ctrl+Shift+F | Only while shown: the box is on the page |
| Portraits of parties and locations (`worldentity` images) | Always fail to load | A picture of the entity as the map shows it | Yes: see [modding](modding.md#portraits-of-world-entities-worldentity-images-version-3) |
| Squirrel 3.2's `split` and `format` | Not available | As `split3_2` and `format3_2`; `split` and `format` stay the original's | Yes: see [modding](modding.md#modern-variants-of-the-string-library-split3_2-format3_2-version-2) |
| Telling the engines apart | - | `::ModernEngine` in scripts, `window.ModernEngine` in the UI | Yes, by design |
| Saves | Its own format | The same format by default, in the original's savegames folder: each game loads the other's saves. A map larger than the original allows is saved in bb's own format, with a warning | Only extra sections the original skips |

### Deliberate behaviour changes and trade-offs

| What | The original | bb | Can mods notice? |
| --- | --- | --- | --- |
| Fullscreen | Exclusive: switches the monitor's mode | A borderless window over the monitor on Windows and macOS (and on Wayland); exclusive only on Linux under X11. A smaller mode is scaled to fit | No: scripts see the same video mode |
| The mouse cursor | Drawn into the game's frames | The same picture, shown as the system's cursor, so it never lags behind a slow frame. `ui.software_cursor = true` brings back the drawn one | No |
| Settings | `config.cfg` in the original's folder | `settings.cfg` in bb's own folder; the original's file is never read or written | No |
| Integers | 32-bit: `2147483647 + 1` wraps to a negative number | 64-bit | Only by going past 32 bits, or `>>>` on a negative number |
| Floats in `format` and `tofloat` | Stored in 32 bits, so extreme values round | Keep their 64-bit value | Only with extreme values (`format("%.1f", 16777217)`) |
| `tostring` of an object | An 8-digit address: `(table : 0x0E6C3960)` | A 16-digit address, the same on every OS | Only mods that parse the address |
| A number argument given `null`, a string or a table | The native reads garbage (often a large number) | Reads 0 | Only mods that pass such values (e.g. `Math.min(null, 5)` is 0) |
| Blurred text shadows in the UI | Coherent draws them lighter and wider | Chromium's are adjusted to match: a rule's `text-shadow` reads with twice the blur and 0.65 of the alpha | Yes, through the computed style |
| Tiled UI backgrounds | Coherent draws each tile upside down | Drawn the same, through an extra style rule and a `data-bb-tiled` attribute | Yes, through the page's styles |
| UI scroll bars | Each one checks its content 5 times a second, forever | Updates when its content changes, within a frame | Only by counting the bar's `update` events |
| UI calls that get no answer (`asyncCallSQ`, `registerScreen`, …) | Each stays in coherent.js's table of calls waiting for an answer (`engine._ActiveRequests`) unless one of the original's stray empty replies happens to hit it: the table grows with play | Never answered either (the promise stays pending), but left out of the table, so the UI's memory doesn't grow with every call | Only by reading `engine._ActiveRequests` |
| Velocity's measuring elements (UI animations between units: px, %, em) | Each kept in jQuery's data store for good, with the style read from it: the UI's memory grows a little with every such animation | Let go once measured | No: they are Velocity's own |
| Mouse wheel in a scrolling box of a mod's UI | One notch jumps to the end | One notch scrolls at most the box's height; vanilla's lists scroll as before | Only as a slower scroll |
| `openURL` from the UI | Opens anything the system has a handler for | Opens only web addresses and Steam store pages, and logs refusals | Only mods linking to other kinds of address |
| `isSteamBuild()` | `true` in the Steam build, `false` in the GOG build | Follows the install the game runs from | Yes, as the original's two builds differ |
| Collecting unreachable script objects | Every 60 seconds | When a battle ends, a campaign is left or a save loads, behind the loading screen | Only through weak references' timing |
| Positional sounds' pan (a sound played at a place on the map) | By side only: every sound left of the screen's centre pans alike, 5 units off or 500 | Grows with the sound's distance from the centre of the screen, left to right, up to the original's at the screen's edge; the volume is unchanged | Only mods that move sound positions to work around the original's pan: `::ModernEngine.Version >= 4` tells them apart: see [modding](modding.md#positional-sounds-pan-by-their-place-on-the-screen-version-4) |
| The AI's thinking in battle | On a worker thread while the frame draws | About 5 ms a frame on the game's thread: a long decision can take a few frames more | No: decisions are the same |
| Sprite animations and the scripts' random numbers | Animated swamp bubbles on screen shift what 0-argument `Math.rand()` returns | Animations have their own random numbers | Only through 0-argument `Math.rand()`, which doesn't repeat in the original either |
| Fog of war drift on the world map | Moves every frame | Moves up to 60 times a second, fewer when zoomed far out | No: only when sub-pixel moves land |
| Battle animations (the active unit's hop, clouds, rain, fades) | Drawn at 60 fps | Drawn in 1/60 s steps (longer when nearly still), so a still screen needs no new frames | No: the game's state still moves every frame |
| Mouse moves to the scripts | Each move the system reports, at the next frame after its keys; the system merges a fast mouse's moves to a few a frame (1 to 4 at 60 fps, probed) | The last move of each frame, once, after its keys; a click or the wheel still comes after a move to where it happened | Only by counting `onMouseInput` moves in a frame |
| The UI page when nothing changes | Runs every frame | Rests until it has work: a mouse move can reach it a frame later | Only as frame-sized delays |
| Loaded textures | Every texture loaded is kept for the session | Only what is drawn stays on the GPU; a sprite whose picture isn't ready can show a frame late | No |
| Decoded sounds | Decoded again every time they play | Up to 32 MB of decoded sound is kept | No |
| `log.html` | In `Documents/Battle Brothers`, overwritten at each start | In bb's own folder; earlier sessions are kept zipped | No |

## Performance

Measured on the same machine (Ryzen 7 5800X3D, Radeon RX 9070 XT), the original under
Proton with vanilla and Modern Hooks. Figures for `bb` are from the date of each
measurement; later work has mostly made `bb` faster still.

| What | The original | bb | How measured |
| --- | --- | --- | --- |
| Launch to the main menu | 14 s (11.6 s by its own clock) | 1.10–1.16 s (warm; 1.50 s cold) | The original from launch, under Proton; bb to the menu shown and done animating, 1280 × 720 |
| A new campaign (world generation) | 8.56 s | 2.70 s | The scripts' own clock, headless bb, same seed |
| Loading a save (`World.load`) | 0.58 s | 0.30 s | Headless, same save |
| Saving | 80 ms | 13 ms | Headless, same campaign |
| A battle's start | 2.78 s (loading screen up at 1.89 s) | 2.35 s (up at 1.34 s) | From the combat dialog to the battle running, with renderer and UI |
| Opening the character screen | ~240 ms | ~92 ms | The screen's `show()`, most of it vanilla's JavaScript |
| The Load Campaign list | ~0.14 s for 130 saves | ~75 ms for 120 saves | Warm; the click's frame, most of it vanilla's JavaScript |
| Script benchmarks (calls, tables, strings, the class system) | - | 1.3–3.7 × faster | The same scripts in both engines (2026-10-14) |
| `Math.rand(1, 100)` | ~960 ns a call | ~175 ns | The same benchmark |
| World pathfinding / tile scans | - | 2.8 × / 1.8 × faster | The same benchmark |
| World entity queries (420 calls) | 2.3 ms | 1.0 ms | The same benchmark |
| Idle in a town (vanilla) | 28% of a core at 60 fps | 13.5% at 180 Hz (1920 × 1080) | CPU, % of one core |
| World map fully zoomed out | 60 fps | 180 fps at 180 Hz (1.6 ms of work a frame) | Panning fully zoomed out; bb at 2560 × 1358 |
| Memory on a fresh world | 514 MB | 172 MB | Not like for like: the original's whole process (with Wine, renderer and UI) against bb's engine alone |

Improvements measured against earlier versions of `bb`, with no figure from the
original to compare:

| What | Before | Now | How measured |
| --- | --- | --- | --- |
| Textures on the GPU on a fresh world | 436 MiB (every atlas kept, as the original) | 119 MiB (183 MiB in a battle): only the sprites drawn | 1920 × 1080 |
| The world map's water and fog of war textures | 42.7 MiB (2048² RGBA8 with mips, as the game's files) | 5.3 MiB (block compressed, BC1 and BC4; on screen at most 4 levels of 255 off) | Texel bytes; a GPU without BC compression keeps the fog in one channel (26.7 MiB) |
| Script memory, 32 mods, a player's save | 466 MB | 174 MB | The Squirrel VM's heap |
| Loading every script (vanilla and a large mod stack) | ~470 ms | ~65 ms | With the compiled-script cache |
| Idle at the main menu, 32 mods, 180 Hz | 16.6% of a core | 1.3% | CPU, % of one core |
| A battle's idle turn, 32 mods, 180 Hz | 22.6% of a core | 14.6% | CPU, % of one core |
| A battle with a 1000 Hz mouse moving, 32 mods, 180 Hz | 2.0 ms a frame running the scripts' mouse handlers, between frames (6 moves a frame) | 0.6 ms, in the frame (one move) | `battle_hover` example, 2026-10-23 |
| A 280 × 280 world map, one frame | - | 0.38 ms (0.28 ms at vanilla size) | Headless: a frame's cost doesn't grow with the map |

## Not changed (yet)

Some of the original's odd behaviour is still matched on purpose, because mods may rely
on it (for example, contract "surprising help" screens returning after a load, or the
original save format's limits on very long strings). Any of it that changes later will
be listed here.
