# Building from source

You don't need to build `bb` to play it or to make mods for it: the published packages
are enough. Build it yourself only to change the engine, or to try a change before it is
released.

Besides Rust, the build compiles some C and C++ code: the Squirrel VM and the wrapper
around Chromium (CEF). To do that it needs a C++ compiler and a few build tools. The
build doesn't install them. If one is missing, it stops with an error that is hard to
read; [When the build fails](#when-the-build-fails) lists the usual errors.

## Windows

Windows 10 or 11, 64-bit. Run all of this in **PowerShell**; you don't need a "Developer
Command Prompt".

### 1. Install the tools

You need these, all free:

| Tool | Why |
|---|---|
| Git | to download the code and its submodules |
| Rust (rustup) | the compiler for most of the engine |
| Visual Studio Build Tools, with "Desktop development with C++" | Microsoft's C++ compiler, linker and the Windows SDK |
| CMake | builds the Squirrel VM and CEF's wrapper |
| Ninja | CEF's wrapper is built with Ninja (CMake alone isn't enough) |
| LLVM | its `libclang.dll` reads the Squirrel VM's C headers |

The quickest way to install them is `winget`, which comes with Windows 10 and 11. Paste
these lines into PowerShell. They can take a while, and Windows may ask you to allow
each installer:

```powershell
winget install --id Git.Git -e
winget install --id Rustlang.Rustup -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Kitware.CMake -e
winget install --id Ninja-build.Ninja -e
winget install --id LLVM.LLVM -e
```

If you already have Visual Studio 2022 (Community, Professional or Build Tools), you
don't need the third line. Check that the **Desktop development with C++** workload is
installed: open the Visual Studio Installer, choose *Modify*, and tick it if it isn't.

If `winget` isn't there, or you'd rather install things by hand, download each one:

- Git: <https://git-scm.com/download/win>
- Rust: <https://rustup.rs> (`rustup-init.exe`; accept the defaults)
- Visual Studio Build Tools: <https://visualstudio.microsoft.com/downloads/> (under
  "Tools for Visual Studio"). In the installer, tick **Desktop development with C++**.
- CMake: <https://cmake.org/download/> (the Windows x64 installer). When it asks, choose
  to add CMake to the system PATH.
- Ninja: <https://github.com/ninja-build/ninja/releases> (`ninja-win.zip`). Unzip
  `ninja.exe` into a folder and add that folder to your PATH. One way is *Settings ›
  System › About › Advanced system settings › Environment Variables*, then edit `Path`
  under "User variables".
- LLVM: <https://github.com/llvm/llvm-project/releases> (the `LLVM-<version>-win64.exe`
  installer). Keep the default folder, `C:\Program Files\LLVM`, because the build looks
  for it there. If you install it somewhere else, set the environment variable
  `LIBCLANG_PATH` to that folder's `bin` subfolder.

### 2. Check them

**Close PowerShell and open a new one**, so it sees the new PATH. Then run:

```powershell
git --version
cargo --version
cmake --version
ninja --version
Test-Path "C:\Program Files\LLVM\bin\libclang.dll"
```

Each command should print a version, and the last should print `True`. If one says it
isn't recognised, that tool isn't installed or isn't on your PATH. Go back to step 1 for
that tool.

### 3. Download the code

```powershell
git clone --recurse-submodules https://github.com/Enduriel/modern-bb-engine.git
cd modern-bb-engine
```

Clone with Git: GitHub's *Download ZIP* leaves out the Squirrel VM, which is a
submodule, and the build can't work without it. If you cloned without
`--recurse-submodules`, run `git submodule update --init --recursive` in the folder.

### 4. Build and run

```powershell
cargo run --release -p bb-app --features ui
```

The first build takes a while. It installs the pinned Rust version (from
`rust-toolchain.toml`), downloads the crates and Chromium (CEF), and compiles all of
them. Later builds only recompile what changed. The program it builds is
`target\release\bb.exe`.

## Linux

The packages on Debian and Ubuntu (the release builds use these):

```bash
sudo apt-get install git cmake ninja-build clang libclang-dev pkg-config libasound2-dev
```

Install Rust from <https://rustup.rs>. Then clone and build as in steps 3 and 4 above.

## macOS

Install Apple's command line tools (`xcode-select --install`), CMake and Ninja (for
example `brew install cmake ninja`), and Rust from <https://rustup.rs>. Then clone and
build as in steps 3 and 4 above.

## When the build fails

Look for the first `error:` in the output. These are the usual ones:

- **`Unable to find libclang: "couldn't find any valid shared libraries matching:
  ['clang.dll', 'libclang.dll']`** (from `squirrel-sys`). LLVM isn't installed, or it is
  installed somewhere other than `C:\Program Files\LLVM`. Install it (step 1), or set
  `LIBCLANG_PATH` to the folder that holds `libclang.dll`. The warnings just above it
  about `llvm-config` don't matter: they go away once libclang is found.
- **`CMake was unable to find a build program corresponding to "Ninja"`** (from
  `cef-dll-sys`). Ninja isn't installed or isn't on your PATH. Install it (step 1), and
  then open a new PowerShell.
- **`failed to execute command: program not found`, or `is cmake not installed?`**.
  CMake isn't on your PATH. CMake that comes with Visual Studio only works inside its
  Developer Command Prompt, so install CMake separately (step 1).
- **`linker 'link.exe' not found`**, or errors that mention `cl.exe` or the Windows
  SDK. The C++ workload of Visual Studio is missing. Tick *Desktop development with C++*
  in the Visual Studio Installer.
- **Errors about missing files in `squirrel-sys`**, for example `CMakeLists.txt` not
  found. The submodules weren't downloaded. Run `git submodule update --init
  --recursive`.

After fixing one, just run the same `cargo` command again; it carries on from where it
stopped.
