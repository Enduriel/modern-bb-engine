# Battle Brothers on the modern engine (`bb`)

`bb` is a new engine for Battle Brothers. It replaces the game's own program and plays
**your own copy** of the game: its scripts, data, UI and DLCs, unchanged. Nothing of the
game ships with `bb`, so you need Battle Brothers installed (from Steam or GOG).

Your mods work too, and they're on by default: `bb` loads every mod `.zip` in the game's
`data` folder, as the original does (Modern Hooks, mod_hooks, MSU and the mods built on
them included).

## Install and run

1. Unpack the archive for your system anywhere you like, for example
   `bb-x86_64-pc-windows-msvc.zip` (Windows), `bb-x86_64-unknown-linux-gnu.tar.gz`
   (Linux) or `bb-aarch64-apple-darwin.tar.gz` (macOS on Apple Silicon).
2. Double-click `bb.exe` (Windows), `bb` (Linux) or `bb.app` (macOS).

That's all. No command line or setup is needed.

### Finding the game

`bb` finds a **Steam** install by itself, in any of your Steam libraries (on Linux also
Steam's Flatpak and Snap versions). Every other copy, GOG's included, you point it at
once:

- If `bb` doesn't find the game, a window says "Battle Brothers wasn't found" and lists
  where it looked. Type or paste the game's folder there, or pick it with `Browse…`.
  `bb` remembers it and starts.
- Or add a line to `bb`'s settings file (see [Where things are](#where-things-are)) naming
  the game's `data` folder (the game's own folder works too), for example:

  ```text
  install.data_dir = C:\GOG Games\Battle Brothers\data
  ```

**macOS:** Battle Brothers has no macOS version, so Steam on a Mac doesn't install it.
Copy the game's folder from a Windows install (the folder with `data` and `win32` in it)
to your Mac, and choose that folder in the window. The app isn't notarized by Apple, so
macOS may refuse to open it the first time; allow it under System Settings, Privacy &
Security.

### Saves

`bb` saves in the original game's format, in the **original's own savegames folder**,
so both games list and load each other's saves:

- **Windows:** `Documents\Battle Brothers\savegames`, as the original.
- **Linux:** the same folder inside the game's Proton prefix, if the game is a Steam
  install and the original has run there at least once. Otherwise `bb` keeps the saves
  in its own folder (below).
- **macOS:** always `bb`'s own folder (the original doesn't run there).

To keep them somewhere else, add `saves.dir = <folder>` to the settings file. A map
bigger than the original allows (a mod can ask for one) is saved in `bb`'s own format,
which the original can't load; `bb` warns you when that happens.

Only one `bb` at a time can play from a savegames folder; a second one says the game is
already running.

### Where things are

`bb` keeps its settings and logs in a folder of its own, `battle-brothers-modern`. It
never reads or changes the original's `config.cfg`, so its video, sound and control
settings start fresh.

| | Settings file | Logs, and the saves when not in the original's folder |
| --- | --- | --- |
| Windows | `%APPDATA%\battle-brothers-modern\settings.cfg` | `%APPDATA%\battle-brothers-modern\` |
| macOS | `~/Library/Application Support/battle-brothers-modern/settings.cfg` | `~/Library/Application Support/battle-brothers-modern/` |
| Linux | `~/.config/battle-brothers-modern/settings.cfg` | `~/.local/share/battle-brothers-modern/` |

On Linux, `$XDG_CONFIG_HOME` and `$XDG_DATA_HOME` are followed when set. `bb` reads the
settings file when it starts.

## Minimum requirements

The game's UI is drawn by Chromium on the GPU and handed to the game as a GPU buffer.
**A real GPU is required:** there is no software fallback, so software renderers and
virtual machines without a GPU passed through don't work. If something is missing, `bb`
stops at once with a message naming it.

**Windows**

- Windows 10 or 11, 64-bit.
- A GPU with a Direct3D 12 driver: AMD Radeon HD 7000 (GCN 1.0, 2012) or newer, NVIDIA
  GeForce GTX 600 (Kepler, 2012) or newer, Intel 4th generation Core (Haswell, 2013) or
  newer.

**Linux**

- A GPU with a Vulkan driver that can import DMA-BUF buffers
  (`VK_EXT_external_memory_dma_buf`); OpenGL alone isn't enough:
  - AMD Radeon HD 7000 (GCN 1.0, 2012) or newer, on Mesa's RADV with the `amdgpu`
    kernel driver;
  - Intel 5th generation Core (Broadwell, 2014) or newer, on Mesa's ANV;
  - NVIDIA GeForce GTX 750/900 (Maxwell, 2014) or newer, on NVIDIA's driver **515 or
    later** with `nvidia-drm.modeset=1`. Kepler cards (GeForce 600/700) are not
    supported: their last driver is too old.
- A GPU render node (`/dev/dri/renderD*`) and an X11 or Wayland session.
- For the error messages to show in a window: `zenity`, `kdialog` or `xmessage`.

**macOS**

- A Mac with Metal, on macOS 12 Monterey or later. Tested on Apple Silicon; Intel Macs
  should work but haven't been tried.

**Every system**

- CPU: one core is enough; more cores start faster.
- Memory: 4 GB recommended for the whole system (2 GB is the floor). The game itself
  uses about 440 MB at the main menu and about 700 MB on a campaign with 32 mods.
- Disk: the game's own install, plus about 320–390 MB for `bb` unpacked.

What each claim rests on, and what has been tested where:
[docs/requirements.md](docs/requirements.md).

## What's different from the original

`bb` does what the original does, quirks included, so mods keep working. The
differences are deliberate:

- **Faster:** the main menu in about a second instead of 14 seconds, a new campaign's
  world in about a third of the time, quicker loading and saving, and less memory.
- **Fixes crashes and limits of the original's engine,** such as battles with more than
  64 factions and world maps bigger than vanilla's.
- **Runs on Windows, Linux and macOS,** without Steam if need be (achievements work on a
  Steam install).
- **Video settings apply at once,** and the window can be resized. Fullscreen is a
  borderless window (exclusive only on Linux under X11).
- **Text on the map in any language,** where the original shows only Western European
  characters.

The full list, with what mods can notice: [docs/differences.md](docs/differences.md).

## Known limitations

- Only a Steam install is found by itself; point `bb` at any other copy as above.
- On Linux, saves are shared with the original only for a Steam install whose Proton
  prefix exists. With any other copy, `bb` keeps its saves in its own folder unless
  you set `saves.dir`.
- The settings aren't taken over from the original: set your video, sound and controls
  again the first time.
- The Windows ARM64 package has no Steam achievements (Valve's library doesn't exist
  for it).
- Some of the original's odd behaviour is kept on purpose, because mods may rely on it.

## Reporting a problem

Report it where you got `bb`, and send both log files of the session that went wrong:

- `log.html`: the game's log, in the original's format, with script errors;
- `cef.log`: the UI's (Chromium's) log.

Both are in `bb`'s folder for logs (see [Where things are](#where-things-are)). Each
start packs the previous session's logs into the `logs` folder there, as
`log-<time>.zip` and `cef-<time>.zip` (the time it started, in UTC), so after a crash
and a new start, send the newest pair from `logs`. Say which mods you had installed;
if the problem goes away with them removed from the game's `data` folder, say that too.

## For modders

Mods made for the original run unchanged. `bb` also adds a few things mods can use
(fonts for any language, Squirrel 3.2's string functions, portraits of map entities),
which a mod checks for through `::ModernEngine`: see [docs/modding.md](docs/modding.md).
