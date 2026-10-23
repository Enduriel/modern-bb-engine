#!/usr/bin/env python3
"""Run the whole test suite: what `cargo test --workspace`, `cargo test -p bb-ui --features
cef` and `cargo test -p bb-app --features ui` run, the same test binaries, side by side.

Run it inside a resource-limited scope, from anywhere, e.g.:

    systemd-run --user --scope -p CPUQuota=400% -p MemoryMax=10G \
        scripts/test_all.py

The default profile is `test-release` (the workspace's Cargo.toml): release's
optimisation, built incrementally: after an edit to squirrel-bb both builds take ~20 s,
not ~2 min as under `release` (its own target folder, ~3 GB bigger with the incremental
cache). `--release` (or `--profile NAME`) builds with another one.

The tests run through cargo-nextest, the version `NEXTEST_VERSION` pins
(install it with `cargo install cargo-nextest --version 0.9.148 --locked`): each test a
process of its own, `--cpu` at a time across all binaries (a many-test binary such as
combat_scenarios spreads over the cores), slowest first (the last runs' times), with a
timeout and one retry each (`.config/nextest.toml`).

Two builds, both workspace-wide so that every crate below bb-ui and bb-app keeps one
feature set (`-p bb-ui --features cef` resolves features differently and rebuilds
squirrel-bb and bb-engine): the plain one, and one with `bb-ui/cef,bb-app/ui`, from
which only bb-ui's and bb-app's test binaries run (the old cef and ui suites). Then
nextest runs their tests (in each test's package folder, with cargo's environment) in
four lanes, a run of nextest each:

- cpu: the plain build's tests (but window.rs), `--cpu` at a time, then the ui build's
  unit tests of bb-ui and bb-app;
- doc: the doctests (`cargo test --doc`; nextest runs none), beside the cpu lane;
- ui: the browser host's tests (paced in real time, mostly idle), at most 3 at a time
  (nextest's `browser` test group; `--ui` lowers it);
- window: bb-app's window.rs in a headless gamescope, one test at a time (the `window`
  test group), the plain build's and then the ui build's. Each run queues for the game
  display as soon as it could otherwise start (the plain one's wait overlaps the cpu
  lane), hands its turn to its tests (`BB_GAME_QUEUE_TURN`: they wait once, not once
  each) and gives it up when it ends. A turn that hasn't come in an hour fails the run,
  naming the holder.

The ui lane starts once the cpu and doc lanes are done, and the ui build's window.rs
(its frame-rate test is load-sensitive) once the ui lane is too. Not alongside them: the
scope's CPU quota throttles every process in it once the cpu lane uses it up, and their
real-time checks then fail (seen: a page collecting garbage early, CEF's shutdown
trapping). window.rs runs the bb of its own build (`BB_TEST_BIN`): the second build
replaces the first's `target/…/bb`.

The ui build is workspace-wide, so our crates get the same features as under the old
`-p bb-ui --features cef` and `-p bb-app --features ui` suites (compared with
`--unit-graph`); only third-party crates' features unify more widely. What it doesn't
show is that bb-ui and bb-app build on their own: `cargo check -p bb-ui --features cef
--tests` (and bb-app's) does.

The `harness = false` binaries (the browser host's) answer nextest's `--list` with one
test (`bb_engine::harness::list_case`) and run whole; their own `test <case> ... ok`
lines count their cases.

A hung test fails on its own (nextest stops it, with what it started, after its
timeout). A test that fails and then passes on its retry is flaky: the run passes, and
the summary lists it (`failed 1 of 2 tries`). Each run of nextest leads a session of its
own; however the runner ends (done, Ctrl-C, SIGTERM, SIGHUP), the runs still going are
stopped with all they started (gamescope, bb, the browser's processes), so none outlives
it into the next queued run.

Each run's output goes to its own log in `--logs`, and its JUnit report to
`target/nextest/default/`; a summary follows, per binary, and the exit status is nonzero
if any test failed.

Skips are reported, never hidden. A test that can't run on this machine (no Steam
install, mod, GPU or gamescope) fails, unless `BB_ALLOW_SKIPS=1` lets it skip
(`bb_engine::harness::skip`), printing `skipping: <why>`. The JUnit reports keep every
test's output, passing ones' too; the summary lists every `skipping:` line with its
test, and the ignored tests with their reasons (asked of each binary: nextest leaves
them out), and the run fails on a skip unless `BB_ALLOW_SKIPS=1` is set for it too. Wall
times are kept in the target folder and order the next run's tests slowest first.
"""

import argparse
import json
import tomllib
import xml.etree.ElementTree as ET
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
# Wall seconds measured on 2026-10-23 (3 CPUs' quota), for the order of a first run: the
# runs (nextest's, the doctests) and the slowest tests, which nextest starts first.
SEED_TIMES = {
	"cpu": 300, "window": 240, "ui": 135, "window+ui": 130, "doctests": 30, "units+ui": 5,
}
NEXTEST_VERSION = "0.9.148"
NEXTEST_INSTALL = f"cargo install cargo-nextest --version {NEXTEST_VERSION} --locked"


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
WINDOW_UI_AFTER = {"cpu", "doc", "ui"}


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


def stop(jobs, grace=20.0):
	"""Stop the jobs still running, each with all it started: SIGTERM to its process group
	(nextest stops its tests then, each in a group of its own), then SIGKILL, after `grace`
	seconds, for every process still in its session (the job is its session's leader)."""
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
		for pid in session_members(job.proc.pid):
			try:
				os.kill(pid, signal.SIGKILL)
			except ProcessLookupError:
				pass
		try:
			job.proc.kill()
		except ProcessLookupError:
			pass
		job.proc.wait()
	return len(live)


def session_members(sid):
	"""The processes in session `sid` (`/proc/<pid>/stat`'s sixth field)."""
	found = []
	for entry in Path("/proc").iterdir():
		if not entry.name.isdigit():
			continue
		try:
			stat = (entry / "stat").read_text()
		except OSError:
			continue
		# The command (field 2) is in parentheses and may hold spaces.
		fields = stat[stat.rindex(")") + 2:].split()
		if int(fields[3]) == sid:
			found.append(int(entry.name))
	return found


def leave(signum, _frame):
	"""A SIGTERM or SIGHUP ends the runner as Ctrl-C does, through its cleanup."""
	raise SystemExit(128 + signum)


# A skip (`bb_engine::harness::skip`): `skipping: <why>`, on a line of its own or after a
# harness=false binary's `test <name> ... `.
SKIP = re.compile(r"^(?:test (\S+) \.\.\. )?skipping: (.*)$")
# libtest's section of a passing test's output (`--show-output`).
SECTION = re.compile(r"^---- (\S+) std(?:out|err) ----$")
# window.rs's one retry of a gamescope that couldn't create its backend (unexplained).
RETRY = re.compile(r"^gamescope couldn't create its backend; trying once more", re.M)
IGNORED = re.compile(r"^test (.+?) \.\.\. ignored(?:, (.*))?$", re.M)


def skips_in(text):
	"""The skips a binary's output reports: (test name or '?', why)."""
	found, test = [], "?"
	for line in text.splitlines():
		if m := SECTION.match(line):
			test = m.group(1)
		elif m := SKIP.match(line.strip()):
			found.append((m.group(1) or test, m.group(2)))
	return found


def harness_false_tests(meta):
	"""The (package, target) names of the test targets with `harness = false`: they take
	no libtest flags."""
	found = set()
	for package in meta["packages"]:
		try:
			manifest = tomllib.loads(Path(package["manifest_path"]).read_text())
		except (OSError, ValueError):
			continue
		for test in manifest.get("test", []):
			if test.get("harness") is False:
				found.add((package["name"], test["name"]))
	return found


RESULT = re.compile(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored")
# A harness=false binary's own lines (bb-ui's and bb-app's print libtest's per-test line).
CASE = re.compile(r"^test \S+ \.\.\. (ok|FAILED|ignored)$", re.M)


def log_name(job):
	return job.label.replace(":", "-").replace("/", "_") + ".log"


class Job:
	"""One run: nextest's over some binaries of a build (`junit` is its report), or the
	doctests'."""

	def __init__(self, label, lane, cmd, cwd, env, est, after=(), junit=None, ui=False):
		self.label, self.lane, self.cmd, self.cwd, self.env = label, lane, cmd, cwd, env
		self.est, self.after = est, set(after)
		self.junit, self.ui = junit, ui
		self.proc = self.log = self.turn = None
		self.start = self.wall = None
		self.rc = None


def check_nextest():
	"""Exit, saying how to install it, unless the pinned cargo-nextest is installed."""
	try:
		out = subprocess.run(["cargo", "nextest", "--version"], cwd=WORKSPACE, stdout=subprocess.PIPE,
			stderr=subprocess.DEVNULL, text=True).stdout
	except OSError:
		out = ""
	found = re.search(r"cargo-nextest (\S+)", out)
	if not found or found.group(1) != NEXTEST_VERSION:
		have = f"cargo-nextest {found.group(1)} is installed" if found else "cargo-nextest isn't installed"
		sys.exit(f"{have}; the suite pins {NEXTEST_VERSION}: {NEXTEST_INSTALL}")


def binary_id(artifact):
	"""nextest's name for a binary: the package for its library, `<package>::<test>` for
	an integration test, `<package>::<kind>/<target>` for the others."""
	name, _ = package_name_version(artifact)
	kind, target = artifact["target"]["kind"][0], artifact["target"]["name"]
	if kind in ("lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"):
		return name
	return f"{name}::{target}" if kind == "test" else f"{name}::{kind}/{target}"


def any_binary(names):
	return " | ".join(f"binary(={n})" for n in names)


def priorities(times, limit=60):
	"""nextest overrides that start the slowest tests first (as the runner started the
	slowest binaries first): the `limit` longest from the last runs, priority 1–100 by
	their wall time. Tests with none keep 0, and run after them."""
	slow = sorted(((t, key) for key, t in times.items() if key.startswith("test ") and t >= 2), reverse=True)[:limit]
	lines = []
	for t, key in slow:
		_, binary_id, test = key.split(" ", 2)
		priority = max(1, min(100, round(t)))
		lines += ["[[profile.default.overrides]]",
			f"filter = {json.dumps(f'binary_id(={binary_id}) & test(={test})')}", f"priority = {priority}", ""]
	return "\n".join(lines)


def nextest_job(label, lane, filterset, features, threads, profile_args, store, times, env, after=()):
	"""A nextest run over the tests `filterset` picks from a build (with `features`); its
	JUnit report goes to the store's `junit-<label>.xml`, through a tool config of its own
	(the repository's config sets no report path), with the slowest tests' priorities."""
	name = label.replace("+", "-")
	tool = store / f"test_all-{name}.toml"
	store.mkdir(parents=True, exist_ok=True)
	tool.write_text(f'[profile.default.junit]\npath = "junit-{name}.xml"\n\n{priorities(times)}\n')
	cmd = ["cargo", "nextest", "run", "--workspace", *profile_args, "--no-tests=pass",
		"--test-threads", str(threads), "--tool-config-file", f"test_all:{tool}", "-E", filterset]
	if features:
		cmd += ["--features", features]
	junit = store / "default" / f"junit-{name}.xml"
	junit.unlink(missing_ok=True)
	return Job(label, lane, cmd, WORKSPACE, env, times.get(label, 0), after=after, junit=junit, ui=bool(features))


def junit_cases(job):
	"""The tests of a nextest job's report: (binary id, test, status, every try's output,
	the last try's, failed tries, seconds); status is ok, FAILED or flaky (it failed, then
	passed on a retry)."""
	try:
		root = ET.parse(job.junit).getroot()
	except (OSError, ET.ParseError):
		return None
	cases = []
	for suite in root.iter("testsuite"):
		for case in suite.iter("testcase"):
			failed = case.find("failure") is not None or case.find("error") is not None
			flaky = case.findall("flakyFailure")
			reruns = case.findall("rerunFailure")
			tries = flaky + reruns
			def text(attempt):
				return "\n".join(el.text or "" for tag in ("system-out", "system-err") for el in attempt.findall(tag))
			status = "FAILED" if failed else "flaky" if flaky else "ok"
			# Every try's output, and the last one's (the result's: its skips, cases, failure).
			cases.append((suite.get("name"), case.get("name"), status, "\n".join(text(a) for a in [*tries, case]),
				text(case), len(tries) + (1 if failed else 0), float(case.get("time") or 0)))
	return cases


def ignored_tests(artifact, env):
	"""A libtest binary's ignored tests with their reasons: its `--list --ignored`, then a
	run of just those (`--exact`, without `--ignored`), which prints each one's reason and
	runs none (nextest doesn't print them)."""
	path = artifact["executable"]
	cwd = Path(artifact["manifest_path"]).parent
	listed = subprocess.run([path, "--list", "--format", "terse", "--ignored"], cwd=cwd, env=env,
		stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
	names = [line[:-len(": test")] for line in listed.stdout.splitlines() if line.endswith(": test")]
	if not names:
		return []
	shown = subprocess.run([path, "--exact", *names], cwd=cwd, env=env, stdout=subprocess.PIPE,
		stderr=subprocess.DEVNULL, text=True)
	return [(m.group(1), m.group(2) or "no reason given") for m in IGNORED.finditer(shown.stdout)]


def main():
	ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
	profile = ap.add_mutually_exclusive_group()
	profile.add_argument("--profile", default="test-release",
		help="cargo's build profile (default test-release: release's optimisation, incremental)")
	profile.add_argument("--release", dest="profile", action="store_const", const="release",
		help="the release profile (= --profile release)")
	ap.add_argument("-j", "--jobs", type=int, default=4, help="cargo's build jobs (default 4)")
	ap.add_argument("--cpu", type=int, default=4, help="CPU-bound tests at once (default 4)")
	ap.add_argument("--ui", type=int, default=3, help="browser-host tests at once (default 3, nextest's browser group's limit)")
	ap.add_argument("--logs", type=Path, help="folder for the runs' logs (default: target/test_all_logs)")
	ap.add_argument("--only", nargs="*", default=None, help="run only the binaries with these target names")
	ap.add_argument("--skip", nargs="*", default=[], help="leave out the binaries with these target names")
	args = ap.parse_args()

	check_nextest()
	profile_args = [f"--cargo-profile={args.profile}", f"--build-jobs={args.jobs}"]
	meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version=1", "--no-deps"], cwd=WORKSPACE,
		stdout=subprocess.PIPE, text=True, check=True).stdout)
	# Cargo's folder for a profile: its name, but `debug` for dev (and test, which inherits it).
	target_dir = Path(meta["target_directory"]) / ("debug" if args.profile in ("dev", "test") else args.profile)
	store = Path(meta["target_directory"]) / "nextest"
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

	no_libtest = harness_false_tests(meta)
	allow_skips = os.environ.get("BB_ALLOW_SKIPS") == "1"

	began = time.monotonic()
	build_args = [f"--profile={args.profile}", f"-j{args.jobs}"]
	plain, plain_scripts, plain_bb = build(build_args, None)
	ui, ui_scripts, ui_bb = build(build_args, UI_FEATURES)
	built = time.monotonic() - began

	# The binaries each build runs (as the lanes split them): its test binaries, the ui
	# build's of bb-ui and bb-app only.
	picked = []
	seen = set()
	for variant, artifacts, scripts in (("", plain, plain_scripts), ("+ui", ui, ui_scripts)):
		for artifact in artifacts:
			name, _ = package_name_version(artifact)
			target = artifact["target"]["name"]
			if artifact["executable"] in seen or (variant and name not in UI_PACKAGES):
				continue
			seen.add(artifact["executable"])
			if args.only is not None and target not in args.only or target in args.skip:
				continue
			picked.append((variant, artifact, scripts))

	# The filtersets: nextest's, over the same binaries as the build's lanes.
	chosen = ""
	if args.only is not None:
		chosen += f" & ({any_binary(args.only) or 'none()'})"
	if args.skip:
		chosen += f" & not ({any_binary(args.skip)})"
	window = "package(bb-app) & kind(test) & binary(=window)"
	ui_packages = "(package(bb-ui) | package(bb-app))"
	env = dict(os.environ)
	# Each run, with which of the picked binaries it runs (a run with none is left out:
	# a window.rs run would queue for the display for nothing).
	def lane_of(variant, artifact):
		kind, target = artifact["target"]["kind"][0], artifact["target"]["name"]
		if target == "window" and kind == "test":
			return "window" + variant
		if not variant:
			return "cpu"
		return "ui" if kind == "test" else "units+ui"
	wanted = {lane_of(variant, artifact) for variant, artifact, _ in picked}
	runs = [
		("cpu", "cpu", f"not ({window})", None, args.cpu, env, ()),
		("units+ui", "cpu", f"{ui_packages} & not kind(test)", UI_FEATURES, args.cpu, env, ()),
		("ui", "ui", f"{ui_packages} & kind(test) & not ({window})", UI_FEATURES, args.ui, env, {"cpu", "doc"}),
		("window", "window", window, None, 1, {**env, "BB_TEST_BIN": plain_bb}, ()),
		("window+ui", "window", window, UI_FEATURES, 1, {**env, "BB_TEST_BIN": ui_bb}, WINDOW_UI_AFTER),
	]
	jobs = [nextest_job(label, lane, f"{filterset}{chosen}", features, threads, profile_args, store, times, run_env_, after)
		for label, lane, filterset, features, threads, run_env_, after in runs if label in wanted]
	if args.only is None:
		doc = ["cargo", "test", "--workspace", *build_args, "--doc"]
		jobs.append(Job("doctests", "doc", doc, WORKSPACE, dict(os.environ), times.get("doctests", 30)))

	# One nextest run at a time in the cpu lane (each runs `--cpu` tests at once), the
	# doctests beside it.
	limits = {"cpu": 1, "doc": 1, "ui": 1, "window": 1}
	waiting = sorted(jobs, key=lambda j: -j.est)
	running = []
	finished = []
	print(f"built in {built:.0f} s; running {len(picked) + (args.only is None)} binaries in {len(jobs)} runs "
		f"(logs in {logs})", flush=True)
	run_began = time.monotonic()
	signal.signal(signal.SIGTERM, leave)
	signal.signal(signal.SIGHUP, leave)
	try:
		run_lanes(waiting, running, finished, limits, logs, run_began)
	finally:
		stopped = stop(running)
		if stopped:
			print(f"stopped {stopped} runs still going, with what they started", flush=True)
		for job in jobs:
			if job.turn:
				job.turn.give_up()
	total = time.monotonic() - began

	# The binaries' results: (label, failed?), tests counted, skips, ignored, flaky tests.
	# Every picked binary counts, those with no test run too (all ignored, or none).
	binaries = {f"{binary_id(artifact)}{variant}": False for variant, artifact, _ in picked}
	counts = [0, 0, 0]
	skips, ignored, retries, flaky, failures = [], [], [], [], []
	kept_times = {}
	harness_false_ids = {f"{p}::{t}" for p, t in no_libtest}
	for job in finished:
		kept_times[job.label] = round(job.wall, 1)
		log = (logs / log_name(job)).read_text(errors="replace")
		if job.junit is None:
			# The doctests: cargo test's own output.
			binaries[job.label] = job.rc != 0
			skips += [(job.label, test, why) for test, why in skips_in(log)]
			ignored += [(job.label, m.group(1), m.group(2) or "no reason given") for m in IGNORED.finditer(log)]
			for m in RESULT.finditer(log):
				for i in range(3):
					counts[i] += int(m.group(i + 1))
			if job.rc != 0:
				failures.append((job.label, None, "".join(log.splitlines(keepends=True)[-40:]), logs / log_name(job)))
			continue
		cases = junit_cases(job)
		if cases is None or job.rc not in (0, 100):
			# No report (the build or a listing failed, or nextest was stopped), or an exit
			# that isn't a test's failure: the whole run failed.
			binaries[f"{job.label} (nextest)"] = True
			failures.append((job.label, None, "".join(log.splitlines(keepends=True)[-40:]), logs / log_name(job)))
			if cases is None:
				continue
		variant = "+ui" if job.ui else ""
		for bid, test, status, output, last, tries, seconds in cases:
			label = f"{bid}{variant}"
			binaries[label] = binaries.get(label, False) or status == "FAILED"
			kept_times[f"test {bid} {test}"] = round(seconds, 1)
			# The skips and cases of the try that counts, the last; gamescope's retries in any.
			skips += [(label, t if t != "?" else test, why) for t, why in skips_in(last)]
			retries += [label] * len(RETRY.findall(output))
			if status == "flaky":
				flaky.append((label, test, tries))
			if status == "FAILED":
				failures.append((label, test, "".join(last.splitlines(keepends=True)[-40:]), logs / log_name(job)))
			# A harness=false binary is one test to nextest; its own lines count its cases.
			own = CASE.findall(last) if bid in harness_false_ids else []
			if own:
				for case in own:
					counts[("ok", "FAILED", "ignored").index(case)] += 1
			else:
				counts[0 if status != "FAILED" else 1] += 1
	# The ignored tests (nextest leaves them out, and their reasons): asked of the binaries.
	for variant, artifact, scripts in picked:
		name, _ = package_name_version(artifact)
		if (name, artifact["target"]["name"]) in no_libtest:
			continue
		label = f"{binary_id(artifact)}{variant}"
		found = ignored_tests(artifact, run_env(artifact, scripts, sysroot_lib, target_dir))
		ignored += [(label, test, why) for test, why in found]
		counts[2] += len(found)

	try:
		try:
			kept = json.loads(times_file.read_text())
		except (OSError, ValueError):
			kept = {}
		kept.update(kept_times)
		times_file.write_text(json.dumps(kept, indent=1, sort_keys=True))
	except OSError:
		pass
	failed = [label for label, bad in binaries.items() if bad]
	print(f"\n{len(binaries) - len(failed)} of {len(binaries)} binaries passed; tests: {counts[0]} passed, "
		f"{counts[1]} failed, {counts[2]} ignored, {len(skips)} skipped; build {built:.0f} s, "
		f"tests {total - built:.0f} s, total {total:.0f} s")
	if ignored:
		print(f"\nignored ({len(ignored)}):")
		for label, test, why in ignored:
			print(f"  {label}: {test}: {why}")
	if skips:
		verdict = "allowed (BB_ALLOW_SKIPS=1)" if allow_skips else "NOT ALLOWED: the run fails (BB_ALLOW_SKIPS=1 allows them)"
		print(f"\nskipped, {verdict} ({len(skips)}):")
		for label, test, why in skips:
			print(f"  {label}: {test}: {why}")
	else:
		print("\nskipped: none")
	if retries:
		print(f"\ngamescope retried after failing to create its backend ({len(retries)}, see the logs): "
			+ ", ".join(retries))
	if flaky:
		print(f"\nflaky, passed on a retry ({len(flaky)}; nextest's retries, .config/nextest.toml):")
		for label, test, tries in flaky:
			print(f"  {label}: {test}: failed {tries} of {tries + 1} tries")
	for label, test, tail, log in failures:
		what = f"{label}: {test}" if test else label
		print(f"\n--- {what} FAILED; the end of its output (all of it in {log}):")
		print(tail)
	return 1 if failed or (skips and not allow_skips) else 0


if __name__ == "__main__":
	sys.exit(main())
