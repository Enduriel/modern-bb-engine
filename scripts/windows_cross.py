#!/usr/bin/env python3
"""Cross-build the workspace for Windows on Linux, and run what can run under Proton.

    systemd-run --user --scope -p CPUQuota=400% -p MemoryMax=10G \
        modern_bb_engine/scripts/windows_cross.py [--run] [--package]

The target is `x86_64-pc-windows-msvc` (`rustup target add x86_64-pc-windows-msvc`),
built with cargo-xwin: clang-cl compiles the C and C++ (the Squirrel VM, CEF's C++
wrapper), rust-lld links, against Microsoft's CRT and Windows SDK, which cargo-xwin
downloads under Microsoft's licence terms. The
tool's version and the CRT's and SDK's are pinned (`CARGO_XWIN`, `XWIN`); cargo-xwin is
installed into the work folder the first time (`cargo install --locked`), or
`--cargo-xwin PATH` (`BB_CARGO_XWIN`) names one. Needs on the path: clang (cargo-xwin
runs it as clang-cl), llvm-lib, cmake, ninja, llvm-objdump and llvm-symbolizer.

The work folder (`<target dir>/windows-msvc`) keeps the tool, the CRT and SDK
(`xwin/`, ~1.2 GB), CEF's Windows distribution (`cef/`, fetched by cef-dll-sys's build,
~600 MB) and the Proton prefix.

What it does:

1. Builds every test binary and `bb.exe` twice, as `test_all.py` does on Linux: the
   plain build, and the UI build (`bb-ui/cef,bb-app/ui`), both workspace-wide. With a
   static C runtime (`+crt-static`): `bb.exe` needs no Visual C++ runtime installed.
2. Checks each `bb.exe` for fused multiply-adds (`vfmadd…`): the seed-exact scripts need
   the Squirrel VM's float results to be the same on every OS. Only the C runtime's own
   maths functions may have them (`CRT_MATHS`: the UCRT picks an FMA3 path at run time
   on CPUs with it), which the scripts reach only through Squirrel's root `::sin`,
   `::pow`, …; any other function with one fails the check.
3. With `--run`: runs each test binary under Proton's Wine (Proton Experimental, or
   `--proton DIR`), in a headless gamescope (nothing on the desktop), with its sound
   sent to a null sink of its own, in a throwaway prefix in the work folder, with
   Proton's DXVK (D3D11, Chromium's) and vkd3d-proton (D3D12, wgpu's) as `proton run`
   picks them. The plain build's binaries, then the UI build's bb-ui and bb-app ones.
   One at a time, from its package folder as cargo runs it, with `BB_ALLOW_SKIPS=1`:
   what can't run here (window.rs's gamescope harness, which runs Linux's `bb`) is a
   skip, and the summary lists every skip. Then `bb.exe` with the UI starts to the main
   menu and takes a screenshot (`ui-menu.png` in the logs folder). Steam is never
   started: nothing sets a Steam app id, and `bb.exe` runs with `--no-steam`.
4. With `--package`: `bb-package --target x86_64-pc-windows-msvc` (a release build of
   `bb.exe` with the UI, laid out with CEF's files and Steam's library, zipped).

The game data is `BB_DATA_DIR` (default: the game in Steam's `~/.local/share/Steam`), passed to the
Windows side as a `Z:` path. The exit status is nonzero if a build, the check, a binary,
the menu or the package failed.
"""

import argparse
import json
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-msvc"


def rustflags_var(target):
	"""cargo's per-target RUSTFLAGS variable for `target`."""
	return "CARGO_TARGET_" + target.replace("-", "_").upper() + "_RUSTFLAGS"


RUSTFLAGS_VAR = rustflags_var(TARGET)
# cargo-xwin 0.23.1 (2026-08-13), and the MSVC CRT and Windows SDK it fetches.
CARGO_XWIN = "0.23.1"
XWIN = {"XWIN_VERSION": "17", "XWIN_CRT_VERSION": "14.44.17.14", "XWIN_SDK_VERSION": "10.0.26100"}
# A static C runtime (no Visual C++ redistributable needed), and no warnings for the
# CRT's missing PDBs (LNK4099: Microsoft ships its static libraries without them).
RUSTFLAGS = "-C target-feature=+crt-static -C link-arg=/ignore:4099"
UI_FEATURES = "bb-ui/cef,bb-app/ui"
UI_PACKAGES = {"bb-ui", "bb-app"}
# The UCRT's maths functions, which carry FMA3 variants chosen at run time.
CRT_MATHS = re.compile(r"^(?:(?:a?(?:sin|cos|tan)h?|atan2|pow|exp2?|log(?:10|2)?|cbrt|hypot)f?|__\w*fma3\w*)$")
# The same, by where the PDB says a function's code comes from (the UCRT's maths sources;
# ARM64's have more helpers than the names above, `_exp2`, `sin_piby4`, …).
CRT_MATHS_SOURCE = re.compile(r"[\\/]ucrt[\\/]src[\\/]appcrt[\\/]tran[\\/]", re.I)
# The fused multiply-add instructions, by the target's architecture (as llvm-objdump
# prints them): x86's FMA3 (`vfmadd231sd`, …), ARM64's scalar and vector ones (`fmadd`,
# `fnmsub`, `fmla`, …).
FMA_INSTRUCTIONS = {
	"x86_64": re.compile(r"\bv?f(?:n)?m(?:add|sub)\d"),
	"aarch64": re.compile(r"\bf(?:n)?m(?:add|sub)\b|\bfml[as]\b"),
}
# Proton's own D3D, as `proton run` sets it: DXVK for D3D11 (Chromium's ANGLE), vkd3d-proton
# for D3D12 (wgpu). Wine's built-in ones (wined3d, upstream vkd3d) can't share the page's
# textures between the two.
DLL_OVERRIDES = "winemenubuilder.exe=d;d3d11,d3d10core,dxgi,d3d9,d3d12,d3d12core=n"
STEAM = Path.home() / ".local/share/Steam"
DEFAULT_DATA = str(STEAM / "steamapps/common/Battle Brothers/data")
NULL_SINK = "bb_windows_cross_null"
RESULT = re.compile(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored")
SKIP = re.compile(r"^(?:test (\S+) \.\.\. )?skipping: (.*)$")
SECTION = re.compile(r"^---- (\S+) std(?:out|err) ----$")
EXIT = re.compile(r"^WINE_EXIT (\d+)$", re.M)
EXPORT = re.compile(r'^export (\w+)="(.*)";$')


def cargo_xwin(work, given):
	"""The pinned cargo-xwin: `given`, or installed into `work` the first time."""
	tool = Path(given) if given else work / "tools" / "bin" / "cargo-xwin"
	if not tool.exists() and not given:
		print(f"installing cargo-xwin {CARGO_XWIN} into {tool.parent.parent}", flush=True)
		env = dict(os.environ, CARGO_TARGET_DIR=str(work / "tools-build"))
		subprocess.run(["cargo", "install", "--locked", f"cargo-xwin@{CARGO_XWIN}", "--root", str(work / "tools"),
			"-j", "4"], cwd=work, env=env, check=True)
		subprocess.run(["rm", "-rf", str(work / "tools-build")], check=True)
	version = subprocess.run([str(tool), "--version"], stdout=subprocess.PIPE, text=True, check=True).stdout.split()
	if version[-1:] != [CARGO_XWIN]:
		sys.exit(f"{tool} is {' '.join(version)}; the pin is {CARGO_XWIN}")
	return tool


def cross_env(xwin, work, target=TARGET):
	"""cargo's environment for the Windows target: cargo-xwin's (clang-cl, lld-link, the CRT
	and SDK, fetched into the work folder the first time), a static C runtime, and CEF's
	Windows distribution kept in the work folder."""
	env = dict(os.environ, XWIN_CACHE_DIR=str(work / "xwin"), **XWIN)
	out = subprocess.run([str(xwin), "xwin", "env", "--target", target], cwd=WORKSPACE, env=env,
		stdout=subprocess.PIPE, text=True, check=True).stdout
	for line in out.splitlines():
		if m := EXPORT.match(line.strip()):
			env[m.group(1)] = m.group(2)
	var = rustflags_var(target)
	env[var] = f"{env.get(var, '')} {RUSTFLAGS}".strip()
	env.pop("RUSTFLAGS", None)
	env["CEF_PATH"] = str(work / "cef")
	return env


def cargo(args, env):
	cmd = ["cargo", *args]
	print("$", " ".join(cmd), flush=True)
	return subprocess.run(cmd, cwd=WORKSPACE, env=env, stdout=subprocess.PIPE, text=True)


def build(profile, jobs, env, features):
	"""Build the workspace's tests and bb.exe (with `features`); return (test artifacts, bb.exe)."""
	args = ["build", "--workspace", "--tests", "--bins", "--target", TARGET, "--profile", profile, "-j", str(jobs),
		"--message-format=json-render-diagnostics"]
	if features:
		args += ["--features", features]
	proc = cargo(args, env)
	if proc.returncode != 0:
		sys.exit("the Windows build failed" + (f" ({features})" if features else ""))
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


def package_of(test):
	return Path(test["manifest_path"]).parent.name


def fused_multiply_adds(exe, arch="x86_64"):
	"""The FMA instructions in `exe` (built for `arch`), by function (from its PDB):
	(outside the C runtime's maths functions, in them)."""
	out = subprocess.run(["llvm-objdump", "-d", "--no-show-raw-insn", str(exe)], stdout=subprocess.PIPE, text=True,
		check=True).stdout
	fma = FMA_INSTRUCTIONS[arch]
	addresses = [line.split(":")[0].strip() for line in out.splitlines() if fma.search(line)]
	if not addresses:
		return {}, {}
	names = subprocess.run(["llvm-symbolizer", f"--obj={exe}", "--functions=short", "--no-inlines"],
		input="".join(f"0x{a}\n" for a in addresses), stdout=subprocess.PIPE, text=True, check=True).stdout
	# Each address answers with its function's name, its source line, and a blank line.
	outside, crt = {}, {}
	for block in names.strip("\n").split("\n\n"):
		lines = block.strip("\n").splitlines() + ["", ""]
		name, source = lines[0], lines[1]
		if name.startswith("??"):
			name = "?"  # no line information: llvm-symbolizer prints only `??:0:0`
		counts = crt if CRT_MATHS.match(name) or CRT_MATHS_SOURCE.search(source) else outside
		counts[name] = counts.get(name, 0) + 1
	return outside, crt


def vm_libraries(bb):
	"""The Squirrel VM's static libraries the build of `bb` (in `<profile>/`) linked."""
	return sorted(bb.parent.glob("build/squirrel-sys-*/out/lib/*.lib"))


def check_vm_fmas(bb, target=TARGET):
	"""Whether the Squirrel VM's own code (squirrel-sys's libraries) has no FMA at all."""
	fma = FMA_INSTRUCTIONS[target.split("-")[0]]
	libraries = vm_libraries(bb)
	if not libraries:
		print(f"FAIL no squirrel-sys libraries beside {bb} to check")
		return False
	good = True
	for library in libraries:
		out = subprocess.run(["llvm-objdump", "-d", "--no-show-raw-insn", str(library)], stdout=subprocess.PIPE,
			text=True, check=True).stdout
		count = sum(1 for line in out.splitlines() if fma.search(line))
		if count:
			print(f"FAIL {library.name}: {count} fused multiply-adds in the Squirrel VM's code")
			good = False
	if good:
		print(f"ok   the Squirrel VM ({', '.join(l.name for l in libraries)}): no fused multiply-adds")
	return good


def check_fmas(bb, target=TARGET):
	"""Whether `bb` has no FMA instruction where it matters (printing what it found).

	The Squirrel VM's libraries must have none (`check_vm_fmas`). x86_64: none in all of
	`bb` outside the C runtime's maths either. ARM64, where FMA is part of the base
	instruction set: Rust crates fuse explicitly (`mul_add`, NEON intrinsics: kira, glam,
	naga, cosmic-text, by their nearest symbols on 2026-10-21; exactly rounded, so the same
	on every ARM64 CPU), so the rest is listed, not refused."""
	arch = target.split("-")[0]
	vm = check_vm_fmas(bb, target)
	outside, crt = fused_multiply_adds(bb, arch)
	if outside and arch != "x86_64":
		print(f"note {bb.name}: {sum(outside.values())} fused multiply-adds outside the C runtime's maths and the VM: "
			+ ", ".join(f"{name or '?'} ({n})" for name, n in sorted(outside.items())))
		return vm
	if outside:
		print(f"FAIL {bb.name} has fused multiply-adds outside the C runtime's maths (the VM's floats must not fuse): "
			+ ", ".join(f"{name} ({n})" for name, n in sorted(outside.items())))
		return False
	print(f"ok   {bb.name}: no fused multiply-adds outside the C runtime's maths ({sum(crt.values())} in "
		f"{len(crt)} of its functions)")
	return vm


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
	"""Make the throwaway prefix as Proton makes a game's (from its default prefix, with
	DXVK and vkd3d-proton in it)."""
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


def wine(files, prefix, args, cwd, log, timeout, extra_env=None):
	"""Run `args` (a Windows program and its arguments) under Proton's Wine in a headless
	gamescope, its output in `log`; (exit status or None, output)."""
	env = dict(os.environ)
	env.update({
		"WINEPREFIX": str(prefix / "pfx"),
		"WINEDLLPATH": f"{files / 'lib/vkd3d'}:{files / 'lib/wine'}",
		"WINEDEBUG": "-all",
		"WINEDLLOVERRIDES": DLL_OVERRIDES,
		"PULSE_SINK": NULL_SINK,
		**(extra_env or {}),
	})
	# The program's status, then the prefix's Wine processes stopped (a lingering
	# winedevice.exe would keep gamescope up).
	script = '"$0/bin/wine" "$@"; rc=$?; "$0/bin/wineserver" -k; echo "WINE_EXIT $rc"'
	with open(log, "w") as out:
		headless(["sh", "-c", script, str(files), *args], env, cwd, out, timeout)
	text = log.read_text(errors="replace")
	m = EXIT.search(text)
	return (int(m.group(1)) if m else None), text


def run_one(test, files, prefix, timeout, logs):
	"""Run one test binary as cargo would, from its package folder; (status, output)."""
	exe = Path(test["executable"])
	manifest = Path(test["manifest_path"])
	extra = {
		"BB_ALLOW_SKIPS": "1",
		"BB_DATA_DIR": windows_path(os.environ.get("BB_DATA_DIR", DEFAULT_DATA)),
		"CARGO_MANIFEST_DIR": str(manifest.parent),
		# CEF's DLLs (cef-dll-sys puts them in the profile's folder, above `deps`).
		"WINEPATH": windows_path(exe.parent.parent),
	}
	log = logs / (exe.stem + ".log")
	return wine(files, prefix, [str(exe), "--show-output"], manifest.parent, log, timeout, extra)


def run_menu(bb, files, prefix, logs):
	"""`bb.exe` with the UI to the main menu: whether it wrote its screenshot."""
	work = logs / "ui-menu"
	(work / "logs").mkdir(parents=True, exist_ok=True)
	shot = logs / "ui-menu.png"
	shot.unlink(missing_ok=True)
	args = [str(bb), "--no-steam", "--mute", "--data", windows_path(os.environ.get("BB_DATA_DIR", DEFAULT_DATA)),
		"--logs", windows_path(work / "logs"), "--settings", windows_path(work / "settings.cfg"),
		"--saves", windows_path(work / "saves"), "--shot", "300", windows_path(shot)]
	status, text = wine(files, prefix, args, bb.parent, logs / "ui-menu.log", 300)
	ok = status == 0 and shot.exists()
	gpu = next((line for line in text.splitlines() if line.startswith("GPU: ")), "GPU: ?")
	print(f"{'ok  ' if ok else 'FAIL'} bb.exe with the UI to the main menu ({gpu}): "
		+ (f"{shot}" if ok else f"see {logs / 'ui-menu.log'} and {work / 'logs'}"), flush=True)
	return ok


def run_all(tests, bb, files, prefix, timeout, logs):
	"""Run every test binary and the UI's main menu; print a summary; return whether all passed."""
	logs.mkdir(parents=True, exist_ok=True)
	setup_prefix(files, prefix, logs)
	sink = subprocess.run(["pactl", "load-module", "module-null-sink", f"sink_name={NULL_SINK}",
		"sink_properties=device.description=BbWindowsCrossNull"], capture_output=True, text=True, check=True).stdout.strip()
	failed, skips, passed = [], [], 0
	try:
		for test in tests:
			name = f"{test['target']['name']} ({package_of(test)})"
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
		if bb is not None and not run_menu(bb, files, prefix, logs):
			failed.append("bb.exe's main menu")
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


def package(env):
	"""bb-package for Windows (a release build of bb.exe with the UI, laid out and zipped)."""
	proc = subprocess.run(["cargo", "run", "--release", "-p", "bb-package", "--", "--target", TARGET], cwd=WORKSPACE,
		env=env)
	return proc.returncode == 0


def main():
	ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
	ap.add_argument("--profile", default="test-release", help="cargo's build profile (default test-release)")
	ap.add_argument("-j", "--jobs", type=int, default=4, help="cargo's build jobs (default 4)")
	ap.add_argument("--cargo-xwin", default=os.environ.get("BB_CARGO_XWIN"),
		help=f"a cargo-xwin {CARGO_XWIN} (default: installed into the work folder)")
	ap.add_argument("--no-ui", action="store_true", help="only the plain build (no CEF)")
	ap.add_argument("--run", action="store_true", help="run the test binaries and the UI's main menu under Proton's Wine")
	ap.add_argument("--package", action="store_true", help="make the Windows release package (bb-package)")
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
	work = Path(meta["target_directory"]) / "windows-msvc"
	work.mkdir(parents=True, exist_ok=True)
	env = cross_env(cargo_xwin(work, args.cargo_xwin), work)

	tests, bb = build(args.profile, args.jobs, env, None)
	print(f"built {bb} and {len(tests)} test binaries", flush=True)
	good = check_fmas(bb)
	ui_bb = None
	if not args.no_ui:
		ui_tests, ui_bb = build(args.profile, args.jobs, env, UI_FEATURES)
		ui_tests = [t for t in ui_tests if package_of(t) in UI_PACKAGES]
		print(f"built {ui_bb} with the UI and {len(ui_tests)} of its test binaries", flush=True)
		good &= check_fmas(ui_bb)
		tests = sorted(tests, key=lambda t: t["target"]["name"]) + sorted(ui_tests, key=lambda t: t["target"]["name"])
	if args.run:
		if args.only is not None:
			tests = [t for t in tests if t["target"]["name"] in args.only]
		good &= run_all(tests, ui_bb, proton_files(args.proton), work / "prefix", args.timeout, work / "logs")
	if args.package:
		good &= package(env)
	sys.exit(0 if good else 1)


if __name__ == "__main__":
	main()
