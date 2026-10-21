#!/usr/bin/env python3
"""Run the whole test suite: what `cargo test --workspace`, `cargo test -p bb-ui --features
cef` and `cargo test -p bb-app --features ui` run, the same test binaries, side by side.

Run it inside a resource-limited scope, from anywhere, e.g.:

    systemd-run --user --scope -p CPUQuota=400% -p MemoryMax=10G \
        modern_bb_engine/scripts/test_all.py

The default profile is `test-release` (the workspace's Cargo.toml): release's
optimisation, built incrementally: after an edit to squirrel-bb both builds take ~20 s,
not ~2 min as under `release` (its own target folder, ~3 GB bigger with the incremental
cache). `--release` (or `--profile NAME`) builds with another one.

Two builds, both workspace-wide so that every crate below bb-ui and bb-app keeps one
feature set (`-p bb-ui --features cef` resolves features differently and rebuilds
squirrel-bb and bb-engine): the plain one, and one with `bb-ui/cef,bb-app/ui`, from
which only bb-ui's and bb-app's test binaries run (the old cef and ui suites). Then the
binaries run, each as cargo runs it (its package folder, cargo's environment), in three
lanes:

- cpu: the CPU-bound binaries, longest first, `--cpu` at a time;
- ui: the browser host's test binaries (paced in real time, mostly idle), `--ui` (3) at
  a time, longest first;
- window: bb-app's window.rs in a headless gamescope, the plain build's and then the ui
  build's. Each queues for the game display as soon as it could otherwise start (the
  plain one's wait overlaps the cpu lane), hands its turn to the binary
  (`BB_GAME_QUEUE_TURN`: its runs wait once, not once each) and gives it up when the
  binary ends. A turn that hasn't come in an hour fails the binary, naming the holder.

The ui lane starts once the cpu lane is done, and the ui build's window.rs (its
frame-rate test is load-sensitive) once the ui lane is too. Not alongside them: the scope's CPU quota throttles every
process in it once the cpu lane uses it up, and their real-time checks then fail (seen:
a page collecting garbage early, CEF's shutdown trapping). window.rs runs the bb of its
own build (`BB_TEST_BIN`): the second build replaces the first's `target/…/bb`.

The ui build is workspace-wide, so our crates get the same features as under the old
`-p bb-ui --features cef` and `-p bb-app --features ui` suites (compared with
`--unit-graph`); only third-party crates' features unify more widely. What it doesn't
show is that bb-ui and bb-app build on their own: `cargo check -p bb-ui --features cef
--tests` (and bb-app's) does.

Each binary runs in a process group of its own; however the runner ends (done, Ctrl-C,
SIGTERM, SIGHUP), the binaries still running are stopped with all they started
(gamescope, bb, the browser's processes), so none outlives it into the next queued
session's run.

Each binary's output goes to its own log in `--logs`; a summary follows, and the exit
status is nonzero if any binary failed. Wall times are kept in the target folder and
order the next run's lanes longest first.
"""

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parent.parent
UI_FEATURES = "bb-ui/cef,bb-app/ui"
# The packages whose ui-build binaries run (the old `-p bb-ui --features cef` and
# `-p bb-app --features ui` suites); every other crate's are the plain build's.
UI_PACKAGES = {"bb-ui", "bb-app"}
# Wall seconds measured on 2026-10-20, for the order of a first run.
SEED_TIMES = {
	"play_session_ui": 176, "frame_regression": 168, "ui_parity": 75, "combat_scenarios": 46,
	"world_lifetime": 30, "campaign_ui": 26, "dlc_subsets": 26, "parity": 21, "ui_window": 18,
	"reload_memory": 16, "world_size_settings": 16, "second_campaign": 14,
	"reload_combat_memory": 14, "menu_ui": 11, "world_sizes": 11, "smoke": 10, "mods_ui": 9,
	"mods_start_input": 9, "world_cull": 9, "ai_time_slicing": 8,
}


def build(profile_args, features):
	"""Build the workspace's tests; return (test binaries, build-script outputs, a copy of
	this build's bb)."""
	cmd = ["cargo", "test", "--workspace", *profile_args, "--no-run", "--message-format=json-render-diagnostics"]
	if features:
		cmd += ["--features", features]
	print("$", " ".join(cmd), flush=True)
	proc = subprocess.run(cmd, cwd=WORKSPACE, stdout=subprocess.PIPE, text=True)
	if proc.returncode != 0:
		sys.exit(f"build failed: {' '.join(cmd)}")
	tests, scripts, kept = [], {}, None
	for line in proc.stdout.splitlines():
		msg = json.loads(line)
		if msg.get("reason") == "compiler-artifact" and msg.get("executable") and msg["profile"]["test"]:
			tests.append(msg)
		elif msg.get("reason") == "compiler-artifact" and msg["target"]["name"] == "bb" and msg.get("executable"):
			# The next build replaces target/…/bb: keep this build's beside it (its
			# libraries are found there too) for window.rs (`BB_TEST_BIN`).
			bb = Path(msg["executable"])
			kept = bb.with_name(f"bb-test-{'ui' if features else 'plain'}")
			shutil.copy2(bb, kept)
		elif msg.get("reason") == "build-script-executed":
			scripts[msg["package_id"]] = msg
	if kept is None:
		sys.exit(f"no bb binary among the build's artifacts, for window.rs: {' '.join(cmd)}")
	return tests, scripts, str(kept)


def run_env(artifact, scripts, sysroot_lib, target_dir):
	"""The environment cargo runs a test binary in."""
	manifest = Path(artifact["manifest_path"])
	package = artifact["package_id"]
	name, version = package_name_version(artifact)
	env = dict(os.environ)
	major, minor, patch = (version.split("-")[0].split(".") + ["0", "0"])[:3]
	env.update({
		"CARGO_MANIFEST_DIR": str(manifest.parent),
		"CARGO_MANIFEST_PATH": str(manifest),
		"CARGO_PKG_NAME": name,
		"CARGO_PKG_VERSION": version,
		"CARGO_PKG_VERSION_MAJOR": major,
		"CARGO_PKG_VERSION_MINOR": minor,
		"CARGO_PKG_VERSION_PATCH": patch,
	})
	env.setdefault("CARGO", "cargo")
	paths = []
	for script in scripts.values():
		for linked in script.get("linked_paths", []):
			path = linked.split("=", 1)[-1]
			if path not in paths:
				paths.append(path)
	paths += [str(target_dir), str(target_dir / "deps"), sysroot_lib]
	if env.get("LD_LIBRARY_PATH"):
		paths.append(env["LD_LIBRARY_PATH"])
	env["LD_LIBRARY_PATH"] = ":".join(paths)
	if package in scripts and scripts[package].get("out_dir"):
		env["OUT_DIR"] = scripts[package]["out_dir"]
	return env


def package_name_version(artifact):
	"""The package's name and version, from its id (`path+file:///…/bb-ui#0.1.0` or
	`path+file:///…#name@0.1.0`)."""
	tail = artifact["package_id"].rsplit("#", 1)[-1]
	if "@" in tail:
		return tail.split("@", 1)
	return Path(artifact["manifest_path"]).parent.name, tail


QUEUE = Path("/tmp/bb-probe-queue")
# The lanes the ui build's window.rs waits out: its frame-rate test is load-sensitive.
WINDOW_UI_AFTER = {"cpu", "ui"}


class Turn:
	"""A place in the game queue (bb-engine's game_queue, the probe runner's), taken for one
	window.rs binary as soon as it could otherwise start and given up when it ends: the
	binary takes it over (`BB_GAME_QUEUE_TURN`), so its runs wait once, not once each."""

	# As long as window.rs itself waits for a turn (game_queue's `max_wait`).
	MAX_WAIT = 3600

	def __init__(self):
		QUEUE.mkdir(exist_ok=True)
		self.path = QUEUE / f"{time.time_ns():020d}-{os.getpid()}"
		self.path.touch()
		self.checked = 0.0
		self.held = False
		self.queued = time.monotonic()

	def ready(self):
		"""First in line? (looked at twice a second at most) Raises TimeoutError, naming
		the holder, once it has waited `MAX_WAIT` seconds."""
		if not self.held and time.monotonic() - self.checked >= 0.5:
			self.checked = time.monotonic()
			live = []
			for ticket in sorted(QUEUE.iterdir()):
				pid = ticket_pid(ticket)
				if pid is None:
					continue
				if Path(f"/proc/{pid}").exists():
					live.append(ticket)
				else:
					ticket.unlink(missing_ok=True)
			self.held = bool(live) and live[0] == self.path
			waited = time.monotonic() - self.queued
			if self.held:
				print(f"game queue: the display is ours after {waited:.0f} s", flush=True)
			elif waited >= self.MAX_WAIT:
				holder = live[0] if live else None
				raise TimeoutError(f"the game display stayed busy for {waited:.0f} s ({QUEUE}); "
					f"held by {describe_holder(holder) if holder else 'nobody (our ticket is gone)'}")
		return self.held

	def give_up(self):
		self.path.unlink(missing_ok=True)


def ticket_pid(ticket):
	"""The pid in a ticket's name (`<nanoseconds>-<pid>`), or None."""
	try:
		return int(ticket.name.split("-")[1])
	except (IndexError, ValueError):
		return None


def describe_holder(ticket):
	"""The process holding the ticket: its pid and command line."""
	pid = ticket_pid(ticket)
	try:
		cmdline = Path(f"/proc/{pid}/cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace").strip()
	except OSError:
		cmdline = "(gone)"
	return f"pid {pid}: {cmdline or '(no command line)'} [{ticket.name}]"


def run_lanes(waiting, running, finished, limits, logs, run_began):
	"""Start the waiting jobs as their lanes allow (a job waits until the lanes in its
	`after` are done, and window.rs for its turn in the game queue), until all have
	finished."""

	def done(job, rc):
		job.rc, job.wall = rc, time.monotonic() - job.start
		job.log.close()
		if job.turn:
			job.turn.give_up()
		finished.append(job)
		status = "ok" if job.rc == 0 else f"FAILED ({job.rc})"
		print(f"{time.monotonic() - run_began:7.1f}  {job.label}: {status} in {job.wall:.1f} s", flush=True)

	while waiting or running:
		for job in list(running):
			if job.proc.poll() is not None:
				running.remove(job)
				done(job, job.proc.returncode)
		lanes_left = {j.lane for j in waiting + running}
		for job in list(waiting):
			if sum(1 for r in running if r.lane == job.lane) >= limits[job.lane]:
				continue
			if job.after & lanes_left:
				continue
			if job.lane == "window":
				job.turn = job.turn or Turn()
				try:
					ready = job.turn.ready()
				except TimeoutError as e:
					job.log = open(logs / log_name(job), "w")
					print(f"game queue: {e}", file=job.log, flush=True)
					job.start = job.turn.queued
					waiting.remove(job)
					done(job, "no turn")
					continue
				if not ready:
					continue
				job.env["BB_GAME_QUEUE_TURN"] = str(job.turn.path)
			job.log = open(logs / log_name(job), "w")
			job.start = time.monotonic()
			# A group of its own, so what it starts can be stopped with it (`stop`).
			job.proc = subprocess.Popen(job.cmd, cwd=job.cwd, env=job.env, stdout=job.log, stderr=subprocess.STDOUT,
				start_new_session=True)
			waiting.remove(job)
			running.append(job)
		time.sleep(0.1)


def stop(jobs, grace=10.0):
	"""Stop the jobs still running, each with its process group (what it started): SIGTERM,
	then SIGKILL for a group still there after `grace` seconds."""
	live = [j for j in jobs if j.proc is not None and j.proc.poll() is None]
	for job in live:
		try:
			os.killpg(job.proc.pid, signal.SIGTERM)
		except ProcessLookupError:
			pass
	deadline = time.monotonic() + grace
	for job in live:
		try:
			job.proc.wait(timeout=max(0.0, deadline - time.monotonic()))
		except subprocess.TimeoutExpired:
			pass
	for job in live:
		try:
			# The group outlives its leader if what it started still runs.
			os.killpg(job.proc.pid, signal.SIGKILL)
		except ProcessLookupError:
			pass
		job.proc.wait()
	return len(live)


def leave(signum, _frame):
	"""A SIGTERM or SIGHUP ends the runner as Ctrl-C does, through its cleanup."""
	raise SystemExit(128 + signum)


RESULT = re.compile(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored")
# A harness=false binary's own lines (bb-ui's and bb-app's print libtest's per-test line).
CASE = re.compile(r"^test \S+ \.\.\. (ok|FAILED|ignored)$", re.M)


def log_name(job):
	return job.label.replace(":", "-").replace("/", "_") + ".log"


class Job:
	def __init__(self, label, lane, cmd, cwd, env, est, after=()):
		self.label, self.lane, self.cmd, self.cwd, self.env = label, lane, cmd, cwd, env
		self.est, self.after = est, set(after)
		self.proc = self.log = self.turn = None
		self.start = self.wall = None
		self.rc = None


def main():
	ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
	profile = ap.add_mutually_exclusive_group()
	profile.add_argument("--profile", default="test-release",
		help="cargo's build profile (default test-release: release's optimisation, incremental)")
	profile.add_argument("--release", dest="profile", action="store_const", const="release",
		help="the release profile (= --profile release)")
	ap.add_argument("-j", "--jobs", type=int, default=4, help="cargo's build jobs (default 4)")
	ap.add_argument("--cpu", type=int, default=4, help="CPU-bound binaries at once (default 4)")
	ap.add_argument("--ui", type=int, default=3, help="browser-host binaries at once (default 3)")
	ap.add_argument("--logs", type=Path, help="folder for the binaries' logs (default: target/test_all_logs)")
	ap.add_argument("--only", nargs="*", default=None, help="run only the binaries with these target names")
	ap.add_argument("--skip", nargs="*", default=[], help="leave out the binaries with these target names")
	args = ap.parse_args()

	profile_args = [f"--profile={args.profile}", f"-j{args.jobs}"]
	meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version=1", "--no-deps"], cwd=WORKSPACE,
		stdout=subprocess.PIPE, text=True, check=True).stdout)
	# Cargo's folder for a profile: its name, but `debug` for dev (and test, which inherits it).
	target_dir = Path(meta["target_directory"]) / ("debug" if args.profile in ("dev", "test") else args.profile)
	logs = args.logs or Path(meta["target_directory"]) / "test_all_logs"
	logs.mkdir(parents=True, exist_ok=True)
	times_file = Path(meta["target_directory"]) / "test_all_times.json"
	try:
		times = {**SEED_TIMES, **json.loads(times_file.read_text())}
	except (OSError, ValueError):
		times = dict(SEED_TIMES)
	sysroot = subprocess.run(["rustc", "--print", "sysroot"], cwd=WORKSPACE, stdout=subprocess.PIPE, text=True,
		check=True).stdout.strip()
	sysroot_lib = str(Path(sysroot) / "lib")

	began = time.monotonic()
	plain, plain_scripts, plain_bb = build(profile_args, None)
	ui, ui_scripts, ui_bb = build(profile_args, UI_FEATURES)
	built = time.monotonic() - began

	jobs = []
	seen = set()
	for variant, artifacts, scripts, bb in (("", plain, plain_scripts, plain_bb), ("+ui", ui, ui_scripts, ui_bb)):
		for artifact in artifacts:
			path = artifact["executable"]
			name, _ = package_name_version(artifact)
			if path in seen or (variant and name not in UI_PACKAGES):
				continue
			seen.add(path)
			target = artifact["target"]["name"]
			if args.only is not None and target not in args.only or target in args.skip:
				continue
			kind = artifact["target"]["kind"][0]
			label = f"{name}:{kind}:{target}{variant}"
			env = run_env(artifact, scripts, sysroot_lib, target_dir)
			cwd = Path(artifact["manifest_path"]).parent
			est = times.get(label, times.get(target, 0))
			if target == "window":
				env["BB_TEST_BIN"] = bb
				jobs.append(Job(label, "window", [path], cwd, env, est, after=WINDOW_UI_AFTER if variant else ()))
			elif variant and kind == "test":
				jobs.append(Job(label, "ui", [path], cwd, env, est, after={"cpu"}))
			else:
				jobs.append(Job(label, "cpu", [path], cwd, env, est))
	if args.only is None:
		doc = ["cargo", "test", "--workspace", *profile_args, "--doc"]
		jobs.append(Job("doctests", "cpu", doc, WORKSPACE, dict(os.environ), times.get("doctests", 30)))

	limits = {"cpu": args.cpu, "ui": args.ui, "window": 1}
	waiting = sorted(jobs, key=lambda j: -j.est)
	running = []
	finished = []
	print(f"built in {built:.0f} s; running {len(jobs)} binaries (logs in {logs})", flush=True)
	run_began = time.monotonic()
	signal.signal(signal.SIGTERM, leave)
	signal.signal(signal.SIGHUP, leave)
	try:
		run_lanes(waiting, running, finished, limits, logs, run_began)
	finally:
		stopped = stop(running)
		if stopped:
			print(f"stopped {stopped} binaries still running, with what they started", flush=True)
		for job in jobs:
			if job.turn:
				job.turn.give_up()
	total = time.monotonic() - began

	try:
		try:
			kept = json.loads(times_file.read_text())
		except (OSError, ValueError):
			kept = {}
		kept.update({j.label: round(j.wall, 1) for j in finished})
		times_file.write_text(json.dumps(kept, indent=1, sort_keys=True))
	except OSError:
		pass
	failed = [j for j in finished if j.rc != 0]
	counts = [0, 0, 0]
	for job in finished:
		text = (logs / log_name(job)).read_text(errors="replace")
		results = list(RESULT.finditer(text))
		for m in results:
			for i in range(3):
				counts[i] += int(m.group(i + 1))
		if not results:
			for m in CASE.finditer(text):
				counts[("ok", "FAILED", "ignored").index(m.group(1))] += 1
	print(f"\n{len(finished) - len(failed)} of {len(finished)} binaries passed; tests: {counts[0]} passed, "
		f"{counts[1]} failed, {counts[2]} ignored; build {built:.0f} s, tests {total - built:.0f} s, total {total:.0f} s")
	for job in failed:
		log = logs / log_name(job)
		print(f"\n--- {job.label} FAILED ({job.rc}); the end of {log}:")
		print("".join(log.read_text(errors="replace").splitlines(keepends=True)[-40:]))
	return 1 if failed else 0


if __name__ == "__main__":
	sys.exit(main())
