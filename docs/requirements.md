# Minimum requirements (draft)

Draft of the requirements section of the player-facing README. Every
statement is tagged with where it comes from:

- **[measured]**: run on real hardware, with the machine and date named;
- **[driver docs]**: from a driver's or Chromium's published support, not run here;
- **[estimate]**: a judgement from the measurements, not itself measured.

`bb` checks the GPU when it starts. If something on this list is missing, it exits at
once with a message box naming what is missing, and writes the same text to stderr
and to `log.html` (see "When the game refuses to start" below).

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

Not available yet. The UI page has no GPU path on Windows (D3D11 shared
handles still to be written). The game refuses to start there with a message saying so.

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
- `no GPU render node (/dev/dri/renderD*) is visible to the game, …`
- `no GPU could draw to the game's window (…)`
- `the browser engine for the UI couldn't start (…)`
- `Chromium couldn't draw the UI on the GPU (…)`: the GPU passed the checks, but no UI
  frame arrived within 10 s. This is a driver problem the checks can't see in advance.
  Chromium's own log (`cef.log`, beside `log.html`) says why.

The message box uses `zenity`, `kdialog` or `xmessage` on Linux (whichever is
installed), `osascript` on macOS, and `MessageBoxW` on Windows. `BB_ERROR_DIALOG=0`
turns it off. The text always goes to stderr and `log.html` as well.
