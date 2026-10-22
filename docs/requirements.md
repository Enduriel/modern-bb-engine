# Minimum requirements (draft)

Draft of the requirements section of the player-facing README. Every
statement is tagged with where it comes from:

- **[measured]**: run on real hardware, with the machine and date named;
- **[driver docs]**: from a driver's or Chromium's published support, not run here;
- **[estimate]**: a judgement from the measurements, not itself measured.

`bb` checks the GPU when it starts. If something on this list is missing, it exits at
once with a message box naming what is missing, and writes the same text to stderr
and to `log.html` (see "When the game refuses to start" below).

## The game itself

The engine plays the player's own copy of Battle Brothers: its game files, the
`data_*.dat` archives in the game's `data` folder. `bb` finds them by itself, so a
double-click is enough. It takes the first of these that holds `data/data_001.dat`:

1. `--data DIR`, then `BB_DATA_DIR` (for developers). Either may name the `data` folder
   or the game's folder. One that holds no game stops `bb` with that error, on stderr
   and in the log only (a command-line error: see below).
2. The folder the player chose the last time nothing was found (`install.data_dir` in
   bb's `settings.cfg`). If it no longer holds the game, the log says so and the
   search goes on.
3. Steam: Steam's folder (Windows' registry; `~/Library/Application Support/Steam` on
   macOS; on Linux every one of the native, Flatpak and Snap clients'), each of its
   libraries, and the game's folder in the one whose `appmanifest_365360.acf` names
   it. `BB_STEAM_DIR` names Steam's folder instead.

Only Steam's install is found by itself: a GOG copy, or any other,
is chosen in the window below, or named by hand in the settings file with the line
`install.data_dir = <the game's data folder>` (the game's folder works too), for
example `install.data_dir = C:\GOG Games\Battle Brothers\data`. The file is
`battle-brothers-modern/settings.cfg` in `%APPDATA%` on Windows, in
`~/Library/Application Support` on macOS, and in `$XDG_CONFIG_HOME` (else `~/.config`)
on Linux; bb reads it at its next start. The window shows the file's full path.

The log (`log.html` and stderr) says which folder was used and where it came from:
`install: the game's data: … (from Steam)`. A folder found in Steam is found again at
every start, so a game Steam moved to another library is followed. Only a folder the
player chose is kept.

A folder holds the game if `data_001.dat` is in it, in its `data` folder, or in the
`data` folder beside it (the game's `win32` folder, where `BattleBrothers.exe` is).

**macOS:** Battle Brothers has no macOS build, so Steam on a Mac doesn't install it. A
Mac player points `bb` at a copy of the Windows game's files (the game's folder, with
`data` and `win32` in it), chosen in the window below or named in the settings file.
The layout is the Windows one on every OS; the window's example is a folder in the
player's home folder (`~/Games/Battle Brothers`). How the player gets the files is
theirs to choose: the window doesn't describe it.

**Nothing found:** a window says "Battle Brothers wasn't found", lists where `bb`
looked and what was there, shows the settings file's full path and the line to add to
it with an example, and asks for the game's folder: typed or pasted into a box,
or picked with `Browse…`, the system's folder dialog (Windows' own, through Chromium;
`zenity` or `kdialog` on Linux, and without either the box only; AppleScript's
`choose folder` on macOS). A folder without the game is refused there, with why. The
folder chosen is kept, and `bb` starts again with it (on Linux and macOS as the same
process, on Windows as a child the first waits for; if the settings file can't be
written, bb passes the folder to itself as `--chosen-data`, which counts as the folder
chosen before). The folder dialog opens attached to bb's window: on Windows it is the
page's parent window (Chromium owns a windowless page's dialogs by it); on Linux under
X11 the tools get `--attach` with bb's window (kdialog honours it; zenity 4 ignores it);
on macOS the AppleScript activates itself first, so its dialog comes to the front. Closing the window ends `bb`.
Built without the UI, `bb` says where it looked and the settings line in the message
box instead, and ends.

## Why a GPU is required

The game's UI is a web page drawn by Chromium. Chromium draws it on the GPU and hands
each frame to the game as a GPU buffer, which the game draws over the scene without a
copy through the CPU. There is no software fallback. This keeps
full-screen UI animations at the display's rate, but the GPU path is required.

## Linux

**Required:**

- A GPU with a **Vulkan** driver that can import **DMA-BUF** buffers
  (`VK_EXT_external_memory_dma_buf`). OpenGL alone is not enough.
- A DRM render node (`/dev/dri/renderD*`) visible to the game. Chromium allocates its
  GPU buffers there.
- An X11 or Wayland session. The game opens a window.

**Which GPUs that means:**

| vendor | oldest generation | driver | source |
|---|---|---|---|
| AMD | GCN 1.0 (Radeon HD 7000, 2012) | Mesa RADV, with the `amdgpu` kernel driver | [driver docs] GCN 1.0/1.1 cards default to the older `radeon` kernel driver on some kernels and distributions, which has no Vulkan; `amdgpu.si_support=1` / `amdgpu.cik_support=1` switches them |
| AMD | RDNA 4 (RX 9070 XT) | Mesa 26.0.3 RADV | [measured] 2026-10-21 |
| Intel | Broadwell (5th gen Core, 2014) | Mesa ANV | [driver docs] Ivy Bridge and Haswell use Mesa's `hasvk` instead; whether it imports DMA-BUFs here is not verified |
| NVIDIA | Maxwell (GeForce GTX 750 / 900, 2014) | NVIDIA's driver **515 or later**, with `nvidia-drm.modeset=1` | [driver docs] 515.48.07 (May 2022) is the first stable driver with `VK_EXT_external_memory_dma_buf`. Kepler (GeForce 600/700) stops at the 470 legacy driver, so **Kepler is not supported** |
| NVIDIA | any | nouveau (NVK) | not checked |

**Not enough** [measured, 2026-10-21, in a headless gamescope on the RX 9070 XT]:

- **OpenGL only.** No Vulkan driver, or wgpu forced onto GL (`WGPU_BACKEND=gl`).
- **Software renderers:** lavapipe (Vulkan) and llvmpipe (OpenGL).
- **No GPU at all,** for example a VM without a GPU passed through or a container
  without `/dev/dri`. wgpu then finds only lavapipe or llvmpipe.
- [estimate] Virtual GPUs (virtio-gpu/venus, VMware SVGA) are untested. They are
  unlikely to import DMA-BUFs from Chromium.

## macOS

- **Required:** a Mac with Metal. The UI page arrives as an IOSurface that Metal reads
  directly.
- [measured] 2026-10-20: an Apple Silicon Mac mini (M4) on macOS 26 runs the game, its
  tests and the UI.
- [driver docs] **macOS 12 Monterey or later**: Chromium dropped macOS 11 with
  version 139, and the game's CEF 154 is newer than that. A later Chromium may have
  raised this again. Not checked against CEF 154's own notes.
- [estimate] Intel Macs with Metal (2012 and later) should work too. Untested.

## Windows

**Required:**

- **Windows 10 or 11** (64-bit). [driver docs] Chromium dropped Windows 7 and 8 with
  version 110, and the game's CEF 154 is newer. The UI page's import also uses
  `CompareObjectHandles` (Windows 10 1607).
- A GPU with a **Direct3D 12** driver. The game draws through Direct3D 12 on Windows
  (wgpu); Chromium draws the UI with Direct3D 11 and hands each frame over as a shared
  texture, which the game opens on its Direct3D 12 device. `WGPU_BACKEND` naming
  another API makes the game refuse to start (the UI can't reach it).

**Which GPUs that means** [driver docs; nothing run on real Windows yet]:

| vendor | oldest generation | source |
|---|---|---|
| AMD | GCN 1.0 (Radeon HD 7000, 2012) | AMD's Direct3D 12 drivers start at GCN |
| Intel | Haswell (4th gen Core, 2013) | Intel's Direct3D 12 drivers start at Haswell |
| NVIDIA | Kepler (GeForce GTX 600/700, 2012) | NVIDIA's Direct3D 12 drivers start at Kepler (Fermi's never shipped) |

**Not enough:** software renderers (WARP, "Microsoft Basic Render Driver"), and virtual
machines without a GPU passed through (they have only WARP).

[measured] 2026-10-21, not on Windows: the cross-built `bb.exe` under Proton
Experimental (DXVK for Direct3D 11, vkd3d-proton for Direct3D 12) on the RX 9070 XT
draws the main menu with its UI.

## CPU and memory

All figures are [measured] (2026-10-21, Ryzen 7
5800X3D, RX 9070 XT, 1280x720 at 60 Hz). The figures add up all of the game's processes
(the game and Chromium's):

- **CPU:** one core is enough. Under a quota of one core (`CPUQuota=100%`), the menu
  shows 1.16 s (vanilla) and 1.51 s (32 mods) after launch, and a loaded modded world
  idles at ~10% of that core. On one core the start gains little from its overlap. Two
  or more cores start faster.
- **Memory used:**
  - ~435–440 MB (PSS) idle at the main menu, vanilla;
  - ~700 MB on a loaded world with 32 mods;
  - it ran within a hard 1.5 GB cap (no swap) on the GPU path.
- **[estimate] RAM to recommend:** 4 GB for the whole system (game + OS + desktop).
  2 GB is the absolute floor.
- **GPU memory:** ~240–280 MB (VRAM + GTT) at the menu with the UI on the GPU. The
  game keeps up to 1 GiB of textures on a GPU with its own memory, and 384 MiB on one
  that shares the system's RAM (integrated GPUs, the Steam Deck). `render.texture_budget_mib`
  in the settings file changes this.
- **Disk:** the game's own install (Steam or GOG) plus the engine's package.

## Start time

[measured] Time from launch to the main menu shown and done animating (vanilla's own
fade-in is ~270 ms of it). Measured 2026-10-21 on the machine above, 1280x720 at 60 Hz,
3 cores, in a headless gamescope, with other work running (load 7–15):

| | before the start-up work | after it |
|---|---|---|
| warm, vanilla (3 runs) | 1.24–1.40 s | 1.10–1.16 s |
| warm, 32 mods (3 runs) | 1.78–1.85 s | 1.47–1.54 s |
| cold, vanilla (1 run) | 1.85 s | 1.50 s |
| cold, 32 mods (1 run) | 2.83 s | 2.96 s (noise: the cold preload of the mods' scripts dominates) |

"Cold" means the binary's, CEF's and the archives' files were dropped from the page
cache and the script cache was empty.

## When the game refuses to start

The message names the first thing missing:

- `no hardware GPU was found, only the software renderer …`
- `the GPU has no usable Vulkan driver (… runs through Gl only)`
- `the GPU's Vulkan driver (…) can't import DMA-BUF buffers (VK_EXT_external_memory_dma_buf), which the UI needs`
- `the GPU has no usable Direct3D 12 driver (… runs through …)` (Windows)
- `no GPU render node (/dev/dri/renderD*) is visible to the game, …`
- `no GPU could draw to the game's window (…)`
- `the browser engine for the UI couldn't start (…)`
- `Chromium couldn't draw the UI on the GPU (…)`: the GPU passed the checks, but no UI
  frame arrived within 10 s. This is a driver problem the checks can't see in advance.
  Chromium's own log (`cef.log`, beside `log.html`) says why.

Any other error that ends the start (a game folder whose files won't load) is shown the
same way, with the game folder `bb` was running and where the log is; so is a crash.

The command line's errors are not: an unknown argument, a
missing or bad value, and `--data` or `BB_DATA_DIR` naming a folder without the game go
to stderr (and `log.html`, once it's open) only, since a player who double-clicks `bb`
passes no arguments and sets no variables.

The message box uses `zenity`, `kdialog` or `xmessage` on Linux (whichever is
installed), `osascript` on macOS, and `MessageBoxW` on Windows. `BB_ERROR_DIALOG=0`
turns it off. The text always goes to stderr and `log.html` as well. On Linux without
any of the three tools, and without a display (no `DISPLAY` or `WAYLAND_DISPLAY`), the
text is only on stderr and in `log.html`.
