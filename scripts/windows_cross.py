#!/usr/bin/env python3
"""Cross-build the workspace for Windows on Linux, and run what can run under Proton.

    systemd-run --user --scope -p CPUQuota=400% -p MemoryMax=10G \
        modern_bb_engine/scripts/windows_cross.py [--run]

The target is `x86_64-pc-windows-gnullvm` (`rustup target add x86_64-pc-windows-gnullvm`)
with the llvm-mingw toolchain (clang, lld, libc++, mingw-w64's headers and import
libraries, the UCRT): no Microsoft SDK. The toolchain is a pinned release tarball,
fetched once into the target folder and checked against its SHA-256 (`LLVM_MINGW`);
`--toolchain DIR` (or `BB_LLVM_MINGW`) uses one already unpacked.

What it does:

1. Builds every test binary and `bb.exe` (the plain build, without the UI) for Windows,
   with a static C runtime (`+crt-static`: no libc++.dll or libunwind.dll to carry).
2. Checks that the Squirrel VM has no fused multiply-adds (`vfmadd…`) in `bb.exe`: the
   seed-exact scripts need its float results to be the same on every OS.
3. Compile-checks the UI build (`bb-app/ui`, `bb-ui/cef`) for Windows, tests and
   examples too. Only a check: cef-dll-sys builds CEF's C++ wrapper for the msvc target
   only, so its build script is turned off here (its `dox` feature) and nothing links.
4. With `--run`: runs each test binary under Proton's Wine (Proton Experimental, or
   `--proton DIR`), in a headless gamescope (nothing on the desktop), with its sound
   sent to a null sink of its own, in a throwaway prefix in the target folder. One at a
   time, from its package folder as cargo runs it, with `BB_ALLOW_SKIPS=1`: what Windows
   can't do yet (the UI page's GPU path, window.rs's gamescope harness) is a skip, and
   the summary lists every skip. Steam is never started: nothing sets a Steam app id,
   and the tests don't load Steam's library.

The game data is `BB_DATA_DIR` (default: the game in Steam's `~/.local/share/Steam`), passed to the
Windows side as a `Z:` path. The exit status is nonzero if a build, the check or a
binary failed.
"""

import argparse
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-gnullvm"
ENV_TARGET = TARGET.replace("-", "_")
# llvm-mingw (github.com/mstorsjo/llvm-mingw): LLVM 23.1.3, UCRT, for an x86_64 Linux host.
LLVM_MINGW = {
	"name": "llvm-mingw-20261006-ucrt-ubuntu-22.04-x86_64",
	"url": "https://github.com/mstorsjo/llvm-mingw/releases/download/20261006/"
	"llvm-mingw-20261006-ucrt-ubuntu-22.04-x86_64.tar.xz",
	"sha256": "5f9c6ed95b2d4bdb2869a488c5fd5857fbdabcf288a0aa3eb1da43f6a08d8ab4",
}
STEAM = Path.home() / ".local/share/Steam"
DEFAULT_DATA = str(STEAM / "steamapps/common/Battle Brothers/data")
NULL_SINK = "bb_windows_cross_null"
RESULT = re.compile(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored")
SKIP = re.compile(r"^(?:test (\S+) \.\.\. )?skipping: (.*)$")
SECTION = re.compile(r"^---- (\S+) std(?:out|err) ----$")
EXIT = re.compile(r"^WINE_EXIT (\d+)$", re.M)


def fetch_toolchain(cache):
	"""The pinned llvm-mingw, unpacked in `cache` (fetched and checked the first time)."""
	root = cache / LLVM_MINGW["name"]
	if (root / "bin" / "x86_64-w64-mingw32-clang").exists():
		return root
	cache.mkdir(parents=True, exist_ok=True)
	archive = cache / (LLVM_MINGW["name"] + ".tar.xz")
	if not archive.exists():
		print(f"fetching {LLVM_MINGW['url']}", flush=True)
		subprocess.run(["curl", "-fsSL", "-o", str(archive) + ".part", LLVM_MINGW["url"]], check=True)
		os.replace(str(archive) + ".part", archive)
	digest = hashlib.sha256(archive.read_bytes()).hexdigest()
	if digest != LLVM_MINGW["sha256"]:
		archive.unlink()
		sys.exit(f"{archive.name}: SHA-256 {digest}, but the pin is {LLVM_MINGW['sha256']}; refusing it")
	subprocess.run(["tar", "-xJf", str(archive), "-C", str(cache)], check=True)
	return root


def cross_env(toolchain):
	"""cargo's environment for the Windows target: llvm-mingw's compilers and linker."""
	env = dict(os.environ)
	env["PATH"] = f"{toolchain / 'bin'}:{env.get('PATH', '')}"
	env.update({
		f"CC_{ENV_TARGET}": "x86_64-w64-mingw32-clang",
		f"CXX_{ENV_TARGET}": "x86_64-w64-mingw32-clang++",
		f"AR_{ENV_TARGET}": "llvm-ar",
		f"CARGO_TARGET_{ENV_TARGET.upper()}_LINKER": "x86_64-w64-mingw32-clang",
		# A static C runtime, so bb.exe needs no libc++.dll or libunwind.dll beside it.
		# rustc passes `-no-pie` with it, which clang ignores for this target (and warns).
		f"CARGO_TARGET_{ENV_TARGET.upper()}_RUSTFLAGS":
			"-C target-feature=+crt-static -C link-arg=-Wno-unused-command-line-argument",
		# squirrel-sys's bindgen: the target's C headers.
		"BINDGEN_EXTRA_CLANG_ARGS": f"--sysroot={toolchain / 'x86_64-w64-mingw32'}",
	})
	return env


def cargo(args, env):
	cmd = ["cargo", *args]
	print("$", " ".join(cmd), flush=True)
	return subprocess.run(cmd, cwd=WORKSPACE, env=env, stdout=subprocess.PIPE, text=True)


def build(profile, jobs, env):
	"""Build the tests and bb.exe; return (test artifacts, bb.exe)."""
	proc = cargo(["build", "--workspace", "--tests", "--bins", "--target", TARGET, "--profile", profile,
		"-j", str(jobs), "--message-format=json-render-diagnostics"], env)
	if proc.returncode != 0:
		sys.exit("the Windows build failed")
	tests, bb = [], None
	for line in proc.stdout.splitlines():
		msg = json.loads(line)
		if msg.get("reason") != "compiler-artifact" or not msg.get("executable"):
			continue
		if msg["profile"]["test"]:
			tests.append(msg)
		elif msg["target"]["name"] == "bb":
			bb = Path(msg["executable"])
	if bb is None:
		sys.exit("no bb.exe among the build's artifacts")
	return tests, bb


def check_ui(jobs, env):
	"""Compile-check the UI build for Windows (nothing linked: see the module's notes)."""
	env = dict(env, BB_CEF_DIR="C:\\cef-not-downloaded-by-a-check")
	proc = cargo(["check", "-p", "bb-app", "-p", "bb-ui", "--tests", "--examples", "--target", TARGET,
		"-j", str(jobs), "--features", "bb-app/ui,bb-ui/cef,cef-dll-sys/dox",
		"--message-format=json-render-diagnostics"], env)
	return proc.returncode == 0


def fused_multiply_adds(bb, toolchain):
	"""How many FMA instructions bb.exe has."""
	out = subprocess.run([str(toolchain / "bin" / "llvm-objdump"), "-d", "--no-show-raw-insn", str(bb)],
		stdout=subprocess.PIPE, text=True, check=True).stdout
	return len(re.findall(r"\bv?f(?:n)?m(?:add|sub)\d", out))


def windows_path(path):
	"""A Linux path as Wine's `Z:` drive shows it."""
	return "Z:" + str(path).replace("/", "\\")


def proton_files(arg):
	path = Path(arg) if arg else STEAM / "steamapps/common/Proton - Experimental"
	files = path / "files"
	if not (files / "bin" / "wine").exists():
		sys.exit(f"no Proton at {path} (--proton DIR)")
	return files


def headless(cmd, env, cwd, out, timeout):
	"""Run `cmd` in a headless gamescope (nothing on the desktop), then stop all of it."""
	env = dict(env)
	for var in ("DISPLAY", "WAYLAND_DISPLAY", "SteamAppId", "SteamGameId", "SteamOverlayGameId"):
		env.pop(var, None)
	gamescope = ["gamescope", "--backend", "headless", "-W", "1280", "-H", "720", "-w", "1280", "-h", "720", "--"]
	proc = subprocess.Popen(gamescope + cmd, cwd=cwd, env=env, stdout=out, stderr=subprocess.STDOUT,
		stdin=subprocess.DEVNULL, start_new_session=True)
	try:
		proc.wait(timeout)
	except subprocess.TimeoutExpired:
		pass
	finally:
		try:
			os.killpg(proc.pid, signal.SIGKILL)
		except ProcessLookupError:
			pass
		proc.wait()


def setup_prefix(files, prefix, logs):
	"""Make the throwaway prefix as Proton makes a game's (from its default prefix)."""
	if (prefix / "pfx" / "system.reg").exists():
		return
	prefix.mkdir(parents=True, exist_ok=True)
	env = dict(os.environ, STEAM_COMPAT_DATA_PATH=str(prefix), STEAM_COMPAT_CLIENT_INSTALL_PATH=str(STEAM),
		WINEPREFIX=str(prefix / "pfx"))
	script = '"$0" run wineboot; "$1/bin/wineserver" -k'
	with open(logs / "prefix.log", "w") as out:
		headless(["sh", "-c", script, str(files.parent / "proton"), str(files)], env, prefix, out, 300)
	if not (prefix / "pfx" / "system.reg").exists():
		sys.exit(f"Proton made no prefix in {prefix}: see {logs / 'prefix.log'}")


def skips_in(text):
	"""The skips a binary's output reports: (test name or '?', why), as test_all.py reads them."""
	found, test = [], "?"
	for line in text.splitlines():
		if m := SECTION.match(line):
			test = m.group(1)
		elif m := SKIP.match(line.strip()):
			found.append((m.group(1) or test, m.group(2)))
	return found


def run_one(test, files, prefix, timeout, logs):
	"""Run one test binary under Proton's Wine in a headless gamescope; (status, output)."""
	exe = test["executable"]
	manifest = Path(test["manifest_path"])
	env = dict(os.environ)
	env.update({
		"WINEPREFIX": str(prefix / "pfx"),
		"WINEDLLPATH": f"{files / 'lib/vkd3d'}:{files / 'lib/wine'}",
		"WINEDEBUG": "-all",
		"WINEDLLOVERRIDES": "winemenubuilder.exe=d",
		"PULSE_SINK": NULL_SINK,
		"BB_ALLOW_SKIPS": "1",
		"BB_DATA_DIR": windows_path(os.environ.get("BB_DATA_DIR", DEFAULT_DATA)),
		"CARGO_MANIFEST_DIR": str(manifest.parent),
	})
	# The program's status, then the prefix's Wine processes stopped (a lingering
	# winedevice.exe would keep gamescope up).
	script = '"$0/bin/wine" "$@"; rc=$?; "$0/bin/wineserver" -k; echo "WINE_EXIT $rc"'
	log = logs / (Path(exe).stem + ".log")
	with open(log, "w") as out:
		headless(["sh", "-c", script, str(files), exe, "--show-output"], env, manifest.parent, out, timeout)
	text = log.read_text(errors="replace")
	m = EXIT.search(text)
	return (int(m.group(1)) if m else None), text


def run_all(tests, files, prefix, timeout, logs):
	"""Run every test binary; print a summary; return whether all passed."""
	logs.mkdir(parents=True, exist_ok=True)
	setup_prefix(files, prefix, logs)
	sink = subprocess.run(["pactl", "load-module", "module-null-sink", f"sink_name={NULL_SINK}",
		"sink_properties=device.description=BbWindowsCrossNull"], capture_output=True, text=True, check=True).stdout.strip()
	failed, skips, passed = [], [], 0
	try:
		for test in sorted(tests, key=lambda t: t["target"]["name"]):
			name = f"{test['target']['name']} ({Path(test['manifest_path']).parent.name})"
			began = time.monotonic()
			status, text = run_one(test, files, prefix, timeout, logs)
			wall = time.monotonic() - began
			counts = [tuple(map(int, m)) for m in RESULT.findall(text)]
			n_pass = sum(c[0] for c in counts)
			n_fail = sum(c[1] for c in counts)
			skips += [(name, test_name, why) for test_name, why in skips_in(text)]
			ok = status == 0 and n_fail == 0
			passed += n_pass
			print(f"{'ok  ' if ok else 'FAIL'} {name}: {n_pass} passed, {n_fail} failed, {wall:.0f} s"
				+ ("" if status is not None else " (no exit status: timed out or crashed)"), flush=True)
			if not ok:
				failed.append(name)
	finally:
		subprocess.run(["pactl", "unload-module", sink], capture_output=True)
	print(f"\n{passed} tests passed in {len(tests)} binaries; logs in {logs}")
	if skips:
		print(f"{len(skips)} skipped:")
		for name, test, why in skips:
			print(f"  {name} {test}: {why}")
	if failed:
		print("failed: " + ", ".join(failed))
	return not failed


def main():
	ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
	ap.add_argument("--profile", default="test-release", help="cargo's build profile (default test-release)")
	ap.add_argument("-j", "--jobs", type=int, default=4, help="cargo's build jobs (default 4)")
	ap.add_argument("--toolchain", type=Path, default=os.environ.get("BB_LLVM_MINGW"),
		help="an unpacked llvm-mingw (default: the pinned release, fetched into the target folder)")
	ap.add_argument("--run", action="store_true", help="run the test binaries under Proton's Wine")
	ap.add_argument("--proton", help="the Proton folder (default: Steam's Proton - Experimental)")
	ap.add_argument("--timeout", type=float, default=900, help="seconds a binary may run (default 900)")
	ap.add_argument("--only", nargs="*", default=None, help="run only the binaries with these target names")
	args = ap.parse_args()

	installed = subprocess.run(["rustup", "target", "list", "--installed"], cwd=WORKSPACE,
		stdout=subprocess.PIPE, text=True).stdout.split()
	if TARGET not in installed:
		sys.exit(f"the Rust target isn't installed: rustup target add {TARGET}")
	meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version=1", "--no-deps"], cwd=WORKSPACE,
		stdout=subprocess.PIPE, text=True, check=True).stdout)
	work = Path(meta["target_directory"]) / "windows-cross"
	toolchain = args.toolchain or fetch_toolchain(work / "toolchain")
	env = cross_env(Path(toolchain))

	tests, bb = build(args.profile, args.jobs, env)
	print(f"built {bb} and {len(tests)} test binaries", flush=True)
	good = True
	fmas = fused_multiply_adds(bb, Path(toolchain))
	if fmas:
		print(f"FAIL bb.exe has {fmas} fused multiply-adds (the VM's floats must not fuse)")
		good = False
	if check_ui(args.jobs, env):
		print("ok   the UI build compiles for Windows (checked, not linked)")
	else:
		print("FAIL the UI build doesn't compile for Windows")
		good = False
	if args.run:
		if args.only is not None:
			tests = [t for t in tests if t["target"]["name"] in args.only]
		good &= run_all(tests, proton_files(args.proton), work / "prefix", args.timeout, work / "logs")
	sys.exit(0 if good else 1)


if __name__ == "__main__":
	main()
