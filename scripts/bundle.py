#!/usr/bin/env python3
"""Build the release bundle for one target, from a clean checkout, the same way locally and on CI.

    scripts/bundle.py --target TRIPLE [--out DIR] [--cache DIR] [-j N]

Targets: x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-pc-windows-msvc,
aarch64-pc-windows-msvc, aarch64-apple-darwin. Each builds natively on its own OS and
architecture (as the GitHub runners do, `.github/workflows/bundles.yml`); the Windows ones
also cross-build on Linux through cargo-xwin, as `windows_cross.py` does.

1. Checks the inputs: the Rust toolchain `rust-toolchain.toml` names (installed with the
   target's standard library through rustup), `Cargo.lock` current (`--locked`) and
   naming the pinned cef-dll-sys, the squirrel-sys submodule checked out (its commit is
   recorded), and the build tools (cmake, ninja: the pinned release is fetched into the
   cache when none is on the path).
2. Fetches CEF's distribution for the target (`CEF`: one version, each platform's archive
   checked by SHA-256 and SHA-1) into `<cache>/cef`, where cef-dll-sys's build finds it
   (`CEF_PATH`) instead of downloading its own.
3. Sets up the compiler: Windows on Windows, MSVC's environment (vcvarsall, through
   vswhere); Windows on Linux, cargo-xwin's (pinned in `windows_cross.py`, with the MSVC
   CRT and Windows SDK it fetches into `<cache>/windows-msvc`). Windows builds link the
   C runtime statically.
4. Runs `bb-package` (a release build of `bb` with the UI, laid out with CEF's files and
   Steam's pinned library, and archived; see `docs/packaging.md`).
5. Windows: checks `bb.exe` for fused multiply-adds outside the C runtime's maths
   (`windows_cross.check_fmas`; the seed-exact scripts need the same floats everywhere).
6. Smoke check, where the target runs on this machine: the packaged `bb --version`
   (no display needed; a UI build has loaded CEF's library by then).
7. Leaves in `--out` (default `<target dir>/bundles`): `bb-<target>.tar.gz` (Windows:
   `.zip`), `bb-<target>.<ext>.sha256` (`sha256sum -c` reads it), `bb-<target>.inputs.txt`
   (what went into it), and the unpacked `bb-<target>/`.

Every mismatch with a pin stops it. Needs on the path: python 3.11+, rustup, cargo, cmake,
curl, tar; on Linux `strip`; Windows on Linux: clang, llvm-lib, llvm-objdump,
llvm-symbolizer, zip; Windows on Windows: Visual Studio's C++ tools (for the target's
architecture) and llvm-objdump + llvm-symbolizer (the runner's LLVM).
"""

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tomllib
import urllib.parse
import zipfile
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parent.parent
REPO = WORKSPACE
SQUIRREL_SYS = REPO / "squirrel-sys"

TARGETS = {
	# target: (os, arch, CEF's platform name)
	"x86_64-unknown-linux-gnu": ("linux", "x86_64", "linux64"),
	"aarch64-unknown-linux-gnu": ("linux", "aarch64", "linuxarm64"),
	"x86_64-pc-windows-msvc": ("windows", "x86_64", "windows64"),
	"aarch64-pc-windows-msvc": ("windows", "aarch64", "windowsarm64"),
	"aarch64-apple-darwin": ("macos", "aarch64", "macosarm64"),
}

# CEF: the version cef-dll-sys builds against (its version's build metadata), and each
# platform's "minimal" distribution from CEF's CDN: (SHA-256, SHA-1). The SHA-1 is the one
# CEF's index publishes (cef-dll-sys checks it too); the SHA-256 was taken from the
# download (2026-10-21).
CEF_DLL_SYS = "154.5.0+154.0.34"
CEF_VERSION = "154.0.34"
CEF_BUILD = "cef_binary_154.0.34+g14c5a08+chromium-154.0.8037.98"
CEF_CDN = "https://cef-builds.spotifycdn.com"
CEF = {
	"linux64": ("e87e2c76304372dd28b899ceb6efa4b353f52700594ddc358a8237ec07ec6492", "15c91ad767097217b2e82ba8b933fe30b134be69"),
	"linuxarm64": ("4598b350e5a43519dbb9f1c732e8b184333572da53244ff55cc063003c34c5e4", "fa85793519eef199e6b6fa8cc8d6f0eb45c6dac2"),
	"windows64": ("c81c34d048d87276f4a18f0b000206710a1709bb67dd658948b87100e1a538da", "be54bbaf3222191dec59ffd9c9ec7e8a7f44a26d"),
	"windowsarm64": ("a319236e11afa15fc52dad1207e0a0c2e6f2190de545a498a45aa05860c98758", "09c6b6807da289b18704bc6d66f8c24e13562b6d"),
	"macosarm64": ("b7efb24bcce34b94e836a9f545005aea210e08abdf4ee1483ea03b8daf93f080", "b7081f6609e5edccf07be96b5d7977329fbe1bdf"),
}

# Ninja, fetched only when none is on the path: the release's asset for this machine and
# its SHA-256 (taken from the download, 2026-10-21).
NINJA_VERSION = "1.13.2"
NINJA = {
	("linux", "x86_64"): ("ninja-linux.zip", "5749cbc4e668273514150a80e387a957f933c6ed3f5f11e03fb30955e2bbead6"),
	("linux", "aarch64"): ("ninja-linux-aarch64.zip", "fd2cacc8050a7f12a16a2e48f9e06fca5c14fc4c2bee2babb67b58be17a607fc"),
	("macos", "aarch64"): ("ninja-mac.zip", "c99048673aa765960a99cf10c6ddb9f1fad506099ff0a0e137ad8960a88f321b"),
	("windows", "x86_64"): ("ninja-win.zip", "07fc8261b42b20e71d1720b39068c2e14ffcee6396b76fb7a795fb460b78dc65"),
	("windows", "aarch64"): ("ninja-winarm64.zip", "e52f0bdef9dfb1003229dbd6508a508c4073fd017247002adc66e5e806cb0391"),
}

# vcvarsall's names for the architectures (host and target).
VC_ARCH = {"x86_64": "x64", "aarch64": "arm64"}
VC_TOOLS = {"x86_64": "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "aarch64": "Microsoft.VisualStudio.Component.VC.Tools.ARM64"}


def fail(message):
	sys.exit(f"bundle: {message}")


def step(message):
	print(f"== {message}", flush=True)


def run(cmd, env=None, cwd=WORKSPACE, capture=False):
	"""Run `cmd`, failing loudly; its stdout if `capture`."""
	print("$", " ".join(str(c) for c in cmd), flush=True)
	proc = subprocess.run([str(c) for c in cmd], cwd=cwd, env=env, text=True,
		stdout=subprocess.PIPE if capture else None)
	if proc.returncode != 0:
		fail(f"{Path(str(cmd[0])).name} failed (exit status {proc.returncode})")
	return proc.stdout if capture else None


def output(cmd, env=None, cwd=WORKSPACE):
	"""A command's stdout, or None if it can't run or fails."""
	try:
		proc = subprocess.run([str(c) for c in cmd], cwd=cwd, env=env, text=True, stdout=subprocess.PIPE,
			stderr=subprocess.DEVNULL)
	except OSError:
		return None
	return proc.stdout.strip() if proc.returncode == 0 else None


def sha(path, algorithm):
	digest = hashlib.new(algorithm)
	with open(path, "rb") as f:
		while chunk := f.read(1 << 20):
			digest.update(chunk)
	return digest.hexdigest()


def check_hash(path, algorithm, expected):
	actual = sha(path, algorithm)
	if actual != expected:
		fail(f"{path}: {algorithm.upper()} {actual}, but the pin is {expected}; refusing it")


def download(url, to):
	"""Fetch `url` into `to` (through a `.part` file)."""
	part = to.with_name(to.name + ".part")
	to.parent.mkdir(parents=True, exist_ok=True)
	run(["curl", "--fail", "--location", "--silent", "--show-error", "--proto", "=https", "--retry", "3",
		"--output", part, url])
	part.replace(to)


def host():
	"""This machine's (os, arch), the native architecture even under emulation."""
	system = {"Linux": "linux", "Darwin": "macos", "Windows": "windows"}.get(platform.system())
	if system == "windows":
		return system, windows_native_arch()
	machine = platform.machine().lower()
	arch = {"x86_64": "x86_64", "amd64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}.get(machine)
	if system is None or arch is None:
		fail(f"unknown host {platform.system()} {platform.machine()}")
	if system == "macos" and arch == "x86_64" and output(["sysctl", "-n", "sysctl.proc_translated"]) == "1":
		arch = "aarch64"
	return system, arch


def windows_native_arch():
	"""Windows' own architecture (an x64 Python on ARM64 Windows reports AMD64)."""
	import ctypes
	from ctypes import wintypes
	kernel32 = ctypes.windll.kernel32
	process, native = wintypes.USHORT(), wintypes.USHORT()
	if hasattr(kernel32, "IsWow64Process2") and kernel32.IsWow64Process2(kernel32.GetCurrentProcess(),
			ctypes.byref(process), ctypes.byref(native)):
		arch = {0x8664: "x86_64", 0xAA64: "aarch64"}.get(native.value)
		if arch:
			return arch
	return {"AMD64": "x86_64", "ARM64": "aarch64"}.get(os.environ.get("PROCESSOR_ARCHITECTURE", ""), "x86_64")


def toolchain():
	"""The channel rust-toolchain.toml names."""
	with open(WORKSPACE / "rust-toolchain.toml", "rb") as f:
		return tomllib.load(f)["toolchain"]["channel"]


def check_rust(target, env):
	"""The pinned toolchain with the target's standard library (through rustup)."""
	channel = toolchain()
	if not shutil.which("rustup", path=env.get("PATH")):
		fail("rustup isn't on the path (https://rustup.rs); the toolchain comes from rust-toolchain.toml")
	run(["rustup", "toolchain", "install", channel, "--profile", "minimal", "--no-self-update"], env=env)
	run(["rustup", "target", "add", "--toolchain", channel, target], env=env)
	active = output(["rustup", "show", "active-toolchain"], env=env) or ""
	if not active.startswith(channel):
		fail(f"the active toolchain here is {active!r}, not rust-toolchain.toml's {channel} (RUSTUP_TOOLCHAIN set?)")
	return output(["rustc", "-V"], env=env)


def check_lock(env):
	"""Cargo.lock up to date (nothing may change it) and naming the pinned cef-dll-sys."""
	run(["cargo", "metadata", "--locked", "--format-version", "1", "--no-deps"], env=env, capture=True)
	with open(WORKSPACE / "Cargo.lock", "rb") as f:
		lock = tomllib.load(f)
	versions = {p["version"] for p in lock["package"] if p["name"] == "cef-dll-sys"}
	if versions != {CEF_DLL_SYS}:
		fail(f"Cargo.lock has cef-dll-sys {sorted(versions)}, but the CEF pin is for {CEF_DLL_SYS}: update CEF here too")


def squirrel_sys():
	"""The squirrel-sys submodule, and its commit (with its squirrel submodule's)."""
	if not (SQUIRREL_SYS / "Cargo.toml").is_file():
		fail(f"no squirrel-sys at {SQUIRREL_SYS}: check out the submodules (git submodule update --init --recursive)")
	if not (SQUIRREL_SYS / "squirrel" / "CMakeLists.txt").is_file():
		fail(f"squirrel-sys has no squirrel submodule checked out ({SQUIRREL_SYS / 'squirrel'}): git submodule update --init --recursive")
	described = []
	for path in (SQUIRREL_SYS, SQUIRREL_SYS / "squirrel"):
		rev = output(["git", "-C", path, "rev-parse", "HEAD"]) or "unknown"
		dirty = output(["git", "-C", path, "status", "--porcelain", "--untracked-files=no"])
		described.append(rev + (" (modified)" if dirty else ""))
	return described


def ninja(cache, host_os, host_arch, env):
	"""A ninja on the path: the system's, or the pinned release fetched into the cache."""
	found = shutil.which("ninja", path=env.get("PATH"))
	if found:
		return f"{output([found, '--version'])} ({found})"
	asset, sha256 = NINJA.get((host_os, host_arch)) or fail(f"no pinned ninja for {host_os} {host_arch}")
	tools = cache / "tools" / f"ninja-{NINJA_VERSION}"
	exe = tools / ("ninja.exe" if host_os == "windows" else "ninja")
	if not exe.exists():
		archive = cache / "downloads" / f"ninja-{NINJA_VERSION}-{asset}"
		if not archive.exists():
			download(f"https://github.com/ninja-build/ninja/releases/download/v{NINJA_VERSION}/{asset}", archive)
		check_hash(archive, "sha256", sha256)
		with zipfile.ZipFile(archive) as z:
			z.extract(exe.name, tools)
		exe.chmod(0o755)
	env["PATH"] = f"{tools}{os.pathsep}{env['PATH']}"
	return f"{NINJA_VERSION} (pinned, {exe})"


def cef(cache, target):
	"""CEF's archive for the target in `<cache>/cef/<version>/` (fetched once, checked every
	run), as cef-dll-sys looks for it; its folder there once the build has unpacked it."""
	target_os, arch, name = TARGETS[target]
	sha256, sha1 = CEF[name]
	versioned = cache / "cef" / CEF_VERSION
	archive = versioned / f"{CEF_BUILD}_{name}_minimal.tar.bz2"
	if not archive.exists():
		download(f"{CEF_CDN}/{urllib.parse.quote(archive.name)}", archive)
	check_hash(archive, "sha256", sha256)
	check_hash(archive, "sha1", sha1)
	# The unpacked folder is trusted only if it came from this archive (cef-dll-sys
	# writes what it unpacked into archive.json); otherwise it's unpacked again.
	unpacked = versioned / f"cef_{target_os}_{arch}"
	record = unpacked / "archive.json"
	if unpacked.exists():
		try:
			info = json.loads(record.read_text())
			good = info.get("name") == archive.name and info.get("sha1") == sha1
		except (OSError, ValueError):
			good = False
		if not good:
			print(f"{unpacked} isn't from the pinned archive: unpacking it again", flush=True)
			shutil.rmtree(unpacked)
	return archive, unpacked


def msvc_env(target_arch, env):
	"""Visual Studio's environment for building for `target_arch` on this Windows machine."""
	host_arch = windows_native_arch()
	vswhere = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Microsoft Visual Studio/Installer/vswhere.exe"
	if not vswhere.exists():
		fail(f"no vswhere at {vswhere}: Visual Studio (or its Build Tools) with the C++ tools is needed")
	install = output([vswhere, "-latest", "-products", "*", "-requires", VC_TOOLS[target_arch], "-property",
		"installationPath"])
	if not install:
		fail(f"no Visual Studio with {VC_TOOLS[target_arch]} (the C++ tools for {target_arch})")
	vcvars = Path(install) / "VC/Auxiliary/Build/vcvarsall.bat"
	arch = VC_ARCH[target_arch] if host_arch == target_arch else f"{VC_ARCH[host_arch]}_{VC_ARCH[target_arch]}"
	out = subprocess.run(f'"{vcvars}" {arch} >nul && set', shell=True, env=env, text=True, stdout=subprocess.PIPE)
	if out.returncode != 0:
		fail(f"{vcvars} {arch} failed")
	# Windows' variable names ignore case: one spelling each (vcvarsall says `Path`).
	result = {k.upper(): v for k, v in env.items()}
	for line in out.stdout.splitlines():
		key, sep, value = line.partition("=")
		if sep and key:
			result[key.upper()] = value
	# The runner's LLVM: llvm-objdump and llvm-symbolizer for the FMA check, libclang for
	# squirrel-sys's bindgen (not on every image's path).
	llvm = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "LLVM/bin"
	if llvm.is_dir():
		result["PATH"] = f"{result.get('PATH', '')}{os.pathsep}{llvm}"
		result.setdefault("LIBCLANG_PATH", str(llvm))
	print(f"MSVC: {install} ({arch}), VC tools {result.get('VCTOOLSVERSION', '?')}, SDK "
		f"{result.get('WINDOWSSDKVERSION', '?').strip(chr(92))}", flush=True)
	return result


def compiler_env(target, host_os, cache, env, args):
	"""The environment the target's build needs here; a description of the compiler."""
	target_os, arch, _ = TARGETS[target]
	if target_os == "windows":
		import windows_cross
		if host_os == "windows":
			env = msvc_env(arch, env)
			var = windows_cross.rustflags_var(target)
			env[var] = f"{env.get(var, '')} {windows_cross.RUSTFLAGS}".strip()
			return env, "MSVC (native)"
		if host_os != "linux":
			fail(f"{target} builds on Windows, or on Linux through cargo-xwin")
		for tool in ("clang", "llvm-lib", "llvm-objdump", "llvm-symbolizer", "zip"):
			if not shutil.which(tool, path=env.get("PATH")):
				fail(f"cross-building for Windows needs {tool} on the path")
		work = cache / "windows-msvc"
		work.mkdir(parents=True, exist_ok=True)
		saved = os.environ.copy()
		os.environ.clear()
		os.environ.update(env)
		try:
			xwin = windows_cross.cargo_xwin(work, args.cargo_xwin)
			env = windows_cross.cross_env(xwin, work, target)
		finally:
			os.environ.clear()
			os.environ.update(saved)
		pins = " ".join(f"{k}={v}" for k, v in windows_cross.XWIN.items())
		return env, f"cargo-xwin {windows_cross.CARGO_XWIN} ({pins}), clang-cl {output(['clang', '--version']).splitlines()[0]}"
	if host_os != target_os:
		fail(f"{target} builds on {target_os}")
	cc = output(["cc", "--version"], env=env)
	return env, cc.splitlines()[0] if cc else "cc (version unknown)"


def package_version():
	with open(WORKSPACE / "bb-app" / "Cargo.toml", "rb") as f:
		return tomllib.load(f)["package"]["version"]


def smoke(bundle, target):
	"""The packaged bb starts far enough to say what it is."""
	target_os = TARGETS[target][0]
	exe = {"linux": bundle / "bb", "windows": bundle / "bb.exe", "macos": bundle / "bb.app/Contents/MacOS/bb"}[target_os]
	expected = f"bb {package_version()} ({target}, ui)"
	env = dict(os.environ)
	for var in ("DISPLAY", "WAYLAND_DISPLAY"):
		env.pop(var, None)
	print(f"$ {exe} --version", flush=True)
	proc = subprocess.run([str(exe), "--version"], env=env, text=True, capture_output=True, timeout=120)
	if proc.stderr.strip():
		print(proc.stderr.rstrip())
	if proc.returncode != 0 or proc.stdout.strip() != expected:
		fail(f"smoke check: {exe} --version exited {proc.returncode} with {proc.stdout.strip()!r}, not {expected!r}")
	print(f"ok   {proc.stdout.strip()}", flush=True)


def main():
	ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
	ap.add_argument("--target", required=True, choices=sorted(TARGETS))
	ap.add_argument("--out", type=Path, help="where the bundle goes (default <target dir>/bundles)")
	ap.add_argument("--cache", type=Path, help="downloads and tools kept between runs (default <target dir>/bundle-cache)")
	ap.add_argument("-j", "--jobs", type=int, help="cargo's build jobs (default: cargo's)")
	ap.add_argument("--cargo-xwin", default=os.environ.get("BB_CARGO_XWIN"),
		help="Windows on Linux: a cargo-xwin of the pinned version (default: installed into the cache)")
	ap.add_argument("--no-smoke", action="store_true", help="skip running the packaged bb --version")
	args = ap.parse_args()

	target = args.target
	target_os, target_arch, _ = TARGETS[target]
	host_os, host_arch = host()
	target_dir = Path(os.environ.get("CARGO_TARGET_DIR") or WORKSPACE / "target").resolve()
	cache = (args.cache or target_dir / "bundle-cache").resolve()
	out = (args.out or target_dir / "bundles").resolve()
	env = dict(os.environ)
	env.pop("RUSTFLAGS", None)
	env.pop("RUSTUP_TOOLCHAIN", None)
	if args.jobs:
		env["CARGO_BUILD_JOBS"] = str(args.jobs)

	step(f"{target} on {host_os} {host_arch}")
	if target_os == "linux" and (host_os, host_arch) != ("linux", target_arch):
		fail(f"{target} builds on Linux {target_arch} (CI: its own runner); cross-building Linux isn't set up")
	if target_os == "macos" and (host_os, host_arch) != ("macos", "aarch64"):
		fail(f"{target} builds on an Apple Silicon Mac")

	step("inputs")
	rustc = check_rust(target, env)
	check_lock(env)
	squirrel = squirrel_sys()
	code = output(["git", "-C", REPO, "rev-parse", "HEAD"]) or "unknown"
	if output(["git", "-C", REPO, "status", "--porcelain", "--untracked-files=no"]):
		code += " (modified)"
	cmake = output(["cmake", "--version"], env=env)
	if not cmake:
		fail("cmake isn't on the path")
	ninja_version = ninja(cache, host_os, host_arch, env)
	print(f"rustc: {rustc}\nsquirrel-sys: {squirrel[0]}, squirrel {squirrel[1]}\n"
		f"cmake: {cmake.splitlines()[0]}\nninja: {ninja_version}", flush=True)

	step(f"CEF {CEF_VERSION}")
	archive, cef_dir = cef(cache, target)
	print(f"ok   {archive.name}", flush=True)
	env["CEF_PATH"] = str(cache / "cef")

	step("compiler")
	env, compiler = compiler_env(target, host_os, cache, env, args)
	env["CEF_PATH"] = str(cache / "cef")
	env["CARGO_TARGET_DIR"] = str(target_dir)

	step("bb-package")
	out.mkdir(parents=True, exist_ok=True)
	run(["cargo", "run", "--locked", "--release", "-p", "bb-package", "--", "--target", target, "--out", out,
		"--cef-dir", cef_dir, "--cache", cache / "steam"], env=env)
	bundle = out / f"bb-{target}"
	archive_out = out / (f"bb-{target}.zip" if target_os == "windows" else f"bb-{target}.tar.gz")
	if not archive_out.is_file():
		fail(f"bb-package left no {archive_out}")

	if target_os == "windows":
		step("fused multiply-adds")
		import windows_cross
		for tool in ("llvm-objdump", "llvm-symbolizer"):
			if not shutil.which(tool, path=env.get("PATH")):
				fail(f"the FMA check needs {tool} on the path (LLVM)")
		saved = os.environ.copy()
		os.environ["PATH"] = env.get("PATH", "")
		try:
			good = windows_cross.check_fmas(target_dir / target / "release" / "bb.exe", target)
		finally:
			os.environ.clear()
			os.environ.update(saved)
		if not good:
			fail("bb.exe has fused multiply-adds outside the C runtime's maths")

	step("smoke check")
	if args.no_smoke:
		print("skipped (--no-smoke)")
	elif (host_os, host_arch) == (target_os, target_arch):
		smoke(bundle, target)
	else:
		print(f"skipped: {target} doesn't run on {host_os} {host_arch} (windows_cross.py --run runs it under Proton)")

	digest = sha(archive_out, "sha256")
	(out / f"{archive_out.name}.sha256").write_text(f"{digest}  {archive_out.name}\n")
	libraries = [p.name for p in bundle.rglob("*") if p.name in ("libsteam_api.so", "steam_api64.dll", "libsteam_api.dylib")]
	steam = f"{libraries[0]} (steamworks-sys, bb-steam/src/pinned.rs)" if libraries else "none (bb_steam::pinned::WITHOUT_LIBRARY)"
	inputs = [
		f"bundle: {archive_out.name}",
		f"sha256: {digest}",
		f"bb: {package_version()}, code {code}",
		f"squirrel-sys: {squirrel[0]}",
		f"squirrel: {squirrel[1]}",
		f"rustc: {rustc}",
		f"CEF: {archive.name} (sha256 {CEF[TARGETS[target][2]][0]})",
		f"Steam library: {steam}",
		f"compiler: {compiler}",
		f"cmake: {cmake.splitlines()[0]}",
		f"ninja: {ninja_version}",
		f"built on: {host_os} {host_arch}",
	]
	(out / f"bb-{target}.inputs.txt").write_text("\n".join(inputs) + "\n")
	step("done")
	print("\n".join(inputs))


if __name__ == "__main__":
	main()
