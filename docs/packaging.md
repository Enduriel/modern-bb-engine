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
and `strip` on Linux.

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
- `libvk_swiftshader.so`, `libvulkan.so.1`, `vk_swiftshader_icd.json`: the host turns
  CEF's GPU off, and no `bb` process mapped them. Leaving `libvulkan.so.1` out also
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
- **Windows x86_64:** laid out from every DLL and resource at the top of CEF's folder
  (`bb.exe` beside them), but not yet built or run.
- **macOS:** refused. It needs an app bundle with CEF's framework and helper apps.
  The pinned universal `libsteam_api.dylib` is ready for it.

## Steam at run time

`bb` loads Steam's library only on a Steam install (the rule is in
`bb-steam/src/install.rs`): the data folder is `<library>/steamapps/common/<dir>/data`,
and that library's `steamapps/appmanifest_365360.acf` names `<dir>`. On a GOG or any
other install it never loads it. `--no-steam` (or `BB_NO_STEAM=1`) turns Steam off;
`--steam-api PATH` (or `BB_STEAM_API`) loads another copy of the library (for
development). Without Steam running, or without the library, the game runs on with
achievements off, said once in the log (`steam: achievements off: ...`).
