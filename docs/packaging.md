# Release packages

A release package is one folder (and archive) per target: `bb`, the CEF files it runs
the game's UI with, and Steam's library, which `bb` uses for achievements on a Steam
install. `bb-package` builds one:

```bash
cd modern_bb_engine
cargo run --release -p bb-package                                    # for this machine
cargo run --release -p bb-package -- --target x86_64-pc-windows-msvc # another target
```

Options: `--out DIR` (default `<target dir>/package`), `--cache DIR` (where the pinned
crate is kept, default `<target dir>/package-cache`), `--cef-dir DIR` (the CEF
distribution, if the build downloaded more than one), `--no-build` (package the `bb`
already built for the target), `--no-archive`. It needs `curl` and `tar` on the path,
`strip` on Linux, and `codesign` on macOS.

## What it does

1. Builds `bb` (`bb-app` with `ui`) in release for the target, with
   `BB_PACKAGE_RPATH=origin`: on Linux `bb` then looks for `libcef.so` in its own folder
   (`$ORIGIN`) instead of the build's CEF folder. Ordinary builds keep the build's path.
   `bb` finds CEF's resources beside itself when `icudtl.dat` is there, otherwise where
   the build found them.
2. Fetches Steam's library. It is not in the repository, and no `cargo build` or
   `cargo test` downloads it. `bb-package` downloads `steamworks-sys` 0.13.0 from
   crates.io once, and checks it against the pins in `bb-steam/src/pinned.rs`:
   - the SHA-256 of the `.crate` (crates.io's published checksum);
   - the SHA-256 of the target's library inside it.

   A mismatch stops the packaging. The cached copy is checked again on every run.
   Updating the pin is a deliberate change: a new version, new hashes, reviewed.
3. Lays out `<out>/bb-<target>/`:

   | File | From |
   | --- | --- |
   | `bb` (`bb.exe`) | the build, stripped on Linux |
   | `libcef.so`, `icudtl.dat`, `resources.pak`, `chrome_100_percent.pak`, `chrome_200_percent.pak`, `v8_context_snapshot.bin` | CEF's distribution (`libcef.so` stripped) |
   | `locales/en-US.pak` | CEF's English locale only |
   | `libsteam_api.so` / `steam_api64.dll` / `libsteam_api.dylib` | the pinned crate |
   | `licenses/CEF-LICENSE.txt`, `licenses/CEF-CREDITS.html` | CEF's licence and Chromium's notices |
   | `licenses/STEAMWORKS.txt` | where Steam's library comes from, its hash, Valve's terms |

4. Packs it as `<out>/bb-<target>.tar.gz` (Windows: `.zip`, with a `tar` that writes
   zips, as Windows' and macOS's do) and prints both sizes.

## Left out, and why

Checked on Linux (2026-10-19) by running the packaged `bb` with the system locale set to
German and recording every shared object its processes mapped:
- `chrome-sandbox`: the host runs CEF without its sandbox.
- `libvk_swiftshader.so`, `libvulkan.so.1`, `vk_swiftshader_icd.json`: Chromium's GPU
  work (in the game's process) runs through ANGLE on the system's EGL, and
  the game refuses software GPUs at start (`bb_app::requirements`), so SwiftShader is
  never needed. Rechecked 2026-10-21 with the GPU on (the dev build at the main menu,
  not the package): no `bb` process mapped `libvk_swiftshader.so`; the GPU libraries
  mapped were the system's EGL/Mesa and Vulkan drivers. Leaving `libvulkan.so.1` out also
  matters: with `$ORIGIN` as `bb`'s run path, wgpu would otherwise load CEF's Vulkan
  loader instead of the system's.
- Every locale but `en-US`: the page is the game's own, and CEF falls back to `en-US`
  (the menu drew normally with `LANG=de_DE.UTF-8`).
- CEF's debug info: `libcef.so` is 1.4 GB as the "minimal" distribution ships it,
  260 MB stripped.

## Sizes (Linux x86_64, CEF 154.0.34, 2026-10-19)

- `bb` 18 MB stripped (26 MB unstripped); `libcef.so` 260 MB; Steam's library 0.4 MB.
- The package: 321.5 MiB unpacked, 138.7 MiB as `.tar.gz`.

## Targets

- **Linux x86_64:** built and run.
- **Linux aarch64:** a pinned library exists; not built.
- **Windows x86_64 (`x86_64-pc-windows-msvc`):** cross-built and packaged on Linux
  (2026-10-21) with cargo-xwin, which supplies Microsoft's CRT and SDK, clang-cl and
  lld-link. The runner sets that environment up:

  ```bash
  modern_bb_engine/scripts/windows_cross.py --no-ui --package
  ```

  or by hand, with `cargo xwin env --target x86_64-pc-windows-msvc` evaluated in the
  shell, `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS` extended by
  `-C target-feature=+crt-static`, and then `cargo run --release -p bb-package --
  --target x86_64-pc-windows-msvc`. `CEF_PATH` names where CEF's distributions are
  (the runner: `<target>/windows-msvc/cef`); bb-package looks there, as the build does.
  The package: `bb.exe` (19 MB, static C runtime: no Visual C++ runtime needed) beside
  every DLL and resource at the top of CEF's folder, `steam_api64.dll`, the English
  locale; 378.7 MiB, 170.7 MiB as `.zip` (made with `zip` on Linux). The packaged
  `bb.exe` draws the main menu with its UI under Proton; not yet run on real Windows.
  Not yet trimmed as Linux's is: `vk_swiftshader.dll`, `vulkan-1.dll` and their ICD file
  are probably unneeded (the game draws through Direct3D 12 and refuses software GPUs),
  to confirm on Windows before leaving them out.
- **macOS arm64:** built on an M4 (2026-10-20): an app bundle, below. Its executable
  runs the game windowed; launching it through Finder is still to be checked.

## macOS: `bb.app`

```text
bb-aarch64-apple-darwin/
  bb.app/Contents/
    Info.plist
    MacOS/bb, libsteam_api.dylib
    Frameworks/Chromium Embedded Framework.framework   (English locale only)
    Frameworks/bb Helper.app, Frameworks/bb Helper (Renderer).app
  licenses/
```

- CEF's child processes run the helpers, copies of `bb` (each carries its own bundle's
  signature, so not links), and `LSUIElement` keeps them out of the Dock. Chromium
  starts the page's renderer from the `(Renderer)` one; without it the page never
  loads. The `(GPU)`, `(Plugin)` and `(Alerts)` helpers of CEF's own layout go unused:
  the GPU runs in the game's process, and there are no plugins or notifications.
- `bb` finds the framework in `Contents/Frameworks`, and tells CEF the bundle it runs in
  (`bb-ui/src/host/mac.rs`).
- The bundle is signed ad hoc (`codesign --force --deep --sign -`; Apple Silicon runs
  only signed programs). It is not notarized.
- No `strip`: it would break the linker's signature, and the bundle is signed after.
- Sizes (CEF 154.0.34): 334 MiB unpacked, 144 MiB as `.tar.gz`. `bb` is 18 MB, the
  framework 271 MB.

## Steam at run time

`bb` loads Steam's library only on a Steam install (the rule is in
`bb-steam/src/install.rs`): the data folder is `<library>/steamapps/common/<dir>/data`,
and that library's `steamapps/appmanifest_365360.acf` names `<dir>`. On a GOG or any
other install it never loads it. `--no-steam` (or `BB_NO_STEAM=1`) turns Steam off;
`--steam-api PATH` (or `BB_STEAM_API`) loads another copy of the library (for
development). Without Steam running, or without the library, the game runs on with
achievements off, said once in the log (`steam: achievements off: ...`).
