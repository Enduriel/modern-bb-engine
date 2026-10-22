# Release packages

A release package is one folder (and archive) per target: `bb`, the CEF files it runs
the game's UI with, and Steam's library, which `bb` uses for achievements on a Steam
install. `bb-package` builds one:

```bash
cargo run --release -p bb-package                                    # for this machine
cargo run --release -p bb-package -- --target x86_64-pc-windows-msvc # another target
```

For a release, use `scripts/bundle.py` (below): it runs `bb-package` with every input
pinned and checked, the same way here and on CI.

Options: `--out DIR` (default `<target dir>/package`), `--cache DIR` (where the pinned
crate is kept, default `<target dir>/package-cache`), `--cef-dir DIR` (the CEF
distribution, if the build downloaded more than one), `--no-build` (package the `bb`
already built for the target), `--no-archive`. It needs `curl` and `tar` on the path,
`strip` on Linux, and `codesign` on macOS.

## Release bundles: `scripts/bundle.py`

One entry point for every target, run from a checkout with squirrel-sys inside it
(`<repo>/squirrel-sys`, ignored):

```bash
systemd-run --user --scope -p CPUQuota=400% -p MemoryMax=10G \
    scripts/bundle.py --target x86_64-unknown-linux-gnu [--out DIR] [--cache DIR] [-j N]
```

| Target | Builds on | Here (Linux x86_64) |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | Linux x86_64 | native |
| `aarch64-unknown-linux-gnu` | Linux ARM64 | refused (no cross set-up) |
| `x86_64-pc-windows-msvc` | Windows x64 (MSVC), or Linux (cargo-xwin) | cross |
| `aarch64-pc-windows-msvc` | Windows ARM64 or x64 (MSVC), or Linux (cargo-xwin) | cross |
| `aarch64-apple-darwin` | an Apple Silicon Mac | refused |

What it does, failing on the first mismatch:

1. **Inputs.** The toolchain `rust-toolchain.toml` names, installed through rustup with
   the target's standard library; `Cargo.lock` current (`cargo metadata --locked`, and
   `bb-package` runs `--locked`) and naming the cef-dll-sys the CEF pin is for;
   squirrel-sys and its `squirrel` submodule present (their commits are recorded);
   cmake on the path; ninja on the path, or the pinned release (1.13.2, SHA-256 per
   host) fetched into the cache.
2. **CEF.** The target's "minimal" distribution of the pinned version (154.0.34), from
   CEF's CDN into `<cache>/cef/154.0.34/`, checked by SHA-256 (pinned in the script)
   and SHA-1 (CEF's index's); `CEF_PATH` points cef-dll-sys's build there, so it
   unpacks that archive instead of fetching its own. A folder unpacked earlier is kept
   only if its `archive.json` names the pinned archive.
3. **Compiler.** Linux and macOS: the system's. Windows on Windows: Visual Studio's
   environment for the target's architecture (vswhere + `vcvarsall`, cross from x64 to
   ARM64 if need be). Windows on Linux: cargo-xwin with its CRT and SDK, as pinned in
   `windows_cross.py` (into `<cache>/windows-msvc`). Windows links its C runtime
   statically either way.
4. **`bb-package`** for the target, into `--out`, with the CEF folder above and Steam's
   crate cached in `<cache>/steam`.
5. **Windows:** `bb.exe` checked for fused multiply-adds outside the C runtime's maths
   (`windows_cross.check_fmas`, x86's FMA3 or ARM64's `fmadd`/`fmla`/… by target).
6. **Smoke check**, where the target runs on the machine: the packaged `bb --version`
   must print `bb <version> (<target>, ui)`. It needs no display, and a UI build has
   loaded CEF's library by then (on macOS, from the bundle's `Frameworks`).
7. **Leaves** in `--out` (default `<target dir>/bundles`): `bb-<target>.tar.gz`
   (Windows: `.zip`), `bb-<target>.<ext>.sha256` (`sha256sum -c` checks it),
   `bb-<target>.inputs.txt` (the code's and squirrel-sys's commits, rustc, the CEF
   archive, the Steam library, the compiler, cmake and ninja) and the unpacked folder.

`--cache` (default `<target dir>/bundle-cache`) keeps the downloads and tools between
runs. Updating a pin (CEF with cef-dll-sys, ninja, cargo-xwin, Steam's crate in
`bb-steam/src/pinned.rs`) is a deliberate change: new versions, new hashes, reviewed.

### CI: `.github/workflows/bundles.yml`

On demand (`workflow_dispatch`) and on `v*` tags, never on pushes. One job per target
on its own runner, each running `bundle.py` and uploading the archive, its `.sha256`
and `inputs.txt` as the artifact `bb-<target>`:

| Target | Runner |
| --- | --- |
| `x86_64-unknown-linux-gnu` | `ubuntu-24.04` |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` |
| `x86_64-pc-windows-msvc` | `windows-2025` |
| `aarch64-pc-windows-msvc` | `windows-11-arm` |
| `aarch64-apple-darwin` | `macos-15` |

- Third-party actions are pinned by commit (checkout, setup-python, cache,
  upload-artifact). Linux installs cmake, ninja, clang (bindgen's libclang), ALSA's
  headers and the libraries `libcef.so` links (for the smoke check) with apt.
- Caches: cargo's registry and git downloads (keyed on `Cargo.lock`), and the pinned
  downloads (CEF's archive, Steam's crate, ninja; keyed on the files holding the pins).
  Not the target folder: each run builds from scratch.
- **squirrel-sys** comes from one place, the workflow's `SQUIRREL_SYS_REPOSITORY` and
  `SQUIRREL_SYS_REF` (a private fork reads the `SQUIRREL_SYS_TOKEN` secret). The
  repository must hold that commit and its `squirrel` submodule's; until those are
  published, that checkout fails.
- No tests: the suite needs the game's Steam data, which the runners don't have.

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
   | `bb` (`bb.exe`) | the build, stripped on Linux; `bb.exe` carries the game's icon (below) |
   | `libcef.so`, `icudtl.dat`, `resources.pak`, `chrome_100_percent.pak`, `chrome_200_percent.pak`, `v8_context_snapshot.bin` | CEF's distribution (`libcef.so` stripped) |
   | `locales/en-US.pak` | CEF's English locale only |
   | `libsteam_api.so` / `steam_api64.dll` / `libsteam_api.dylib` | the pinned crate |
   | `licenses/CEF-LICENSE.txt`, `licenses/CEF-CREDITS.html` | CEF's licence and Chromium's notices |
   | `licenses/STEAMWORKS.txt` | where Steam's library comes from, its hash, Valve's terms |

4. Packs it as `<out>/bb-<target>.tar.gz` (Windows: `.zip`, with a `tar` that writes
   zips, as Windows' and macOS's do) and prints both sizes.

## The game's icon

`bb` shows the original's icon (`bb-app/assets/bb.ico`, from `BattleBrothers.exe`: 16, 32
and 48 pixels; its README says where it comes from):

- **Windows:** `bb.exe` carries it as its icon resource, for Explorer and the taskbar.
  `bb-app/build.rs` writes the compiled resource (`.res`) itself and hands it to the
  linker (link.exe and lld-link both take one), so no resource compiler is needed, on
  Windows or cross-building. The windows load it from there.
- **Linux, X11:** the windows' `_NET_WM_ICON` (the 48-pixel image).
- **Linux, Wayland:** the windows' toplevels, through the compositor's
  `xdg-toplevel-icon-v1` (KDE has it; GNOME and gamescope don't, and show their default
  icon). For the taskbar's icon by app id a desktop file would be needed; there is none
  yet.
- **macOS:** `bb.app/Contents/Resources/bb.icns` (`CFBundleIconFile`), which
  `bb-package` writes from the same file.

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
- 2026-10-21, from `bundle.py`: `bb` 20.5 MB stripped; the package 323.3 MiB unpacked,
  139.3 MiB as `.tar.gz`, the same files. It drew the main menu (headless gamescope).

## Targets

- **Linux x86_64:** built and run.
- **Linux aarch64:** CEF publishes `linuxarm64` for 154.0.34 (pinned in `bundle.py`)
  and the pinned Steam crate has `linuxarm64/libsteam_api.so`. Not built yet: no cross
  set-up here (it would need an ARM64 sysroot with glibc, libstdc++ and ALSA), so it
  builds on its CI runner (`ubuntu-24.04-arm`).
- **Windows x86_64 (`x86_64-pc-windows-msvc`):** cross-built and packaged on Linux
  (2026-10-21) with cargo-xwin, which supplies Microsoft's CRT and SDK, clang-cl and
  lld-link. The runner sets that environment up:

  ```bash
  scripts/windows_cross.py --no-ui --package
  ```

  or by hand, with `cargo xwin env --target x86_64-pc-windows-msvc` evaluated in the
  shell, `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS` extended by
  `-C target-feature=+crt-static`, and then `cargo run --release -p bb-package --
  --target x86_64-pc-windows-msvc`. `CEF_PATH` names where CEF's distributions are
  (the runner: `<target>/windows-msvc/cef`); bb-package looks there, as the build does.
  `scripts/bundle.py --target x86_64-pc-windows-msvc` does the same with its pins
  (2026-10-21: the packaged `bb.exe` reached the main menu under Proton, and
  `bb.exe --version` ran).
  `bb.exe` is a GUI program (`windows_subsystem`, in every build): double-clicked, no
  console window opens behind the game. Run from a console it writes there (it attaches
  to its parent's), and output redirected to a file or a pipe stays redirected, so
  `bb.exe --version > out.txt` and the smoke check read it as before. A GUI program
  doesn't hold the prompt: in cmd `start /wait bb.exe …`, in PowerShell
  `bb.exe … | Out-Host` waits for it. CEF's child processes run `bb.exe` again, so they
  are GUI programs too (no console of their own), and they end with bb, however bb ends
  (a job object, `bb_app::win_process::job`).
  The package: `bb.exe` (19 MB, static C runtime: no Visual C++ runtime needed) beside
  every DLL and resource at the top of CEF's folder, `steam_api64.dll`, the English
  locale; 378.7 MiB, 170.7 MiB as `.zip` (made with `zip` on Linux). The packaged
  `bb.exe` draws the main menu with its UI under Proton; not yet run on real Windows.
  Not yet trimmed as Linux's is: `vk_swiftshader.dll`, `vulkan-1.dll` and their ICD file
  are probably unneeded (the game draws through Direct3D 12 and refuses software GPUs),
  to confirm on Windows before leaving them out.
- **Windows ARM64 (`aarch64-pc-windows-msvc`):** cross-built and packaged on Linux by
  `bundle.py` (2026-10-21; cargo-xwin fetches the ARM64 CRT and SDK; CEF's
  `windowsarm64`): `bb.exe` 17 MB, 382.7 MiB unpacked, 169.2 MiB as `.zip`. **No Steam
  library:** Valve's SDK in the pinned crate has none for Windows ARM64, and an ARM64
  process can't load the x86_64 one, so the target is listed in
  `bb_steam::pinned::WITHOUT_LIBRARY`; `bb-package` says so, and `STEAMWORKS.txt`
  says why. bb runs there with achievements off. FMA check: the Squirrel VM has none;
  `bb.exe` has 32 outside the C runtime's maths (no line information; by their nearest
  symbols explicit `mul_add`/NEON in Rust crates: kira, glam, naga, cosmic-text), which
  ARM64 rounds the same on every CPU. Never run: no ARM64 Windows here.
- **macOS arm64:** built on an M4 (2026-10-20): an app bundle, below. Its executable
  runs the game windowed; launching it through Finder is still to be checked.

## macOS: `bb.app`

```text
bb-aarch64-apple-darwin/
  bb.app/Contents/
    Info.plist
    MacOS/bb, libsteam_api.dylib
    Resources/bb.icns                                  (the game's icon)
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
