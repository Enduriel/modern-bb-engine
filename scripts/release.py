#!/usr/bin/env python3
"""Make a release: set the version everywhere it is written, commit it and tag it.

    scripts/release.py VERSION [--dry-run]

VERSION is `MAJOR.MINOR.PATCH`, optionally with a pre-release (`0.3.0`, `0.3.0-rc.1`).

1. Checks: on `main`, nothing uncommitted (the submodule included), the tag `vVERSION`
   not taken, VERSION not the current one.
2. Sets `version` in every workspace crate's `Cargo.toml` (they all share one), the
   `bb X.Y.Z (` examples of `bb --version` in the sources, and `Cargo.lock`
   (`cargo update --workspace --offline`).
3. Commits that as `chore: version VERSION` and tags it `vVERSION` (annotated,
   `bb VERSION`).

It pushes nothing: it prints the commands that push the commit and the tag, which start
the bundles workflow (`.github/workflows/bundles.yml`) and its draft release. With
`--dry-run` it shows what it would change and changes nothing. Should a step fail, the
files it changed are restored.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")
BRANCH = "main"


def run(*cmd, capture=True):
	proc = subprocess.run(cmd, cwd=ROOT, text=True, stdout=subprocess.PIPE if capture else None)
	if proc.returncode != 0:
		sys.exit(f"release: `{' '.join(cmd)}` failed ({proc.returncode})")
	return proc.stdout if capture else ""


def packages():
	"""The workspace's crates: (name, version, manifest path)."""
	meta = json.loads(run("cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"))
	return [(p["name"], p["version"], Path(p["manifest_path"])) for p in meta["packages"]]


def checks(version):
	"""The current version, once everything allows `version` to be made."""
	if not VERSION.match(version):
		sys.exit(f"release: {version!r} is not MAJOR.MINOR.PATCH[-PRE]")
	branch = run("git", "branch", "--show-current").strip()
	if branch != BRANCH:
		sys.exit(f"release: on {branch or 'a detached HEAD'}, not {BRANCH}")
	if dirty := run("git", "status", "--porcelain").strip():
		sys.exit(f"release: uncommitted changes:\n{dirty}")
	if run("git", "tag", "--list", f"v{version}").strip():
		sys.exit(f"release: the tag v{version} exists already")
	versions = {v for _, v, _ in packages()}
	if len(versions) != 1:
		sys.exit(f"release: the crates have different versions ({', '.join(sorted(versions))})")
	current = versions.pop()
	if current == version:
		sys.exit(f"release: the version is {version} already")
	return current


def manifest_edit(text, old, new):
	"""`text` (a Cargo.toml) with its [package] version set to `new`."""
	section = re.search(r"(?ms)^\[package\]\n.*?(?=^\[|\Z)", text)
	line = re.compile(rf'(?m)^version = "{re.escape(old)}"$')
	if not section or len(line.findall(section.group(0))) != 1:
		return None
	start, end = section.span()
	return text[:start] + line.sub(f'version = "{new}"', section.group(0)) + text[end:]


def edits(old, new):
	"""Every file to change, with its new text: {path: text}."""
	out = {}
	for name, _, manifest in packages():
		text = manifest_edit(manifest.read_text(), old, new)
		if text is None:
			sys.exit(f'release: no single `version = "{old}"` in [package] of {manifest}')
		out[manifest] = text
	example = f"bb {old} ("
	for rel in run("git", "grep", "-l", "-F", example, "--", ":!Cargo.lock").split():
		path = ROOT / rel
		out[path] = out.get(path, path.read_text()).replace(example, f"bb {new} (")
	return out


def main():
	parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
	parser.add_argument("version", help="the new version, MAJOR.MINOR.PATCH[-PRE]")
	parser.add_argument("--dry-run", action="store_true", help="show the changes, make none")
	args = parser.parse_args()
	new = args.version
	old = checks(new)
	changes = edits(old, new)
	print(f"version {old} -> {new}")
	for path in sorted(changes):
		print(f"  {path.relative_to(ROOT)}")
	print("  Cargo.lock")
	if args.dry_run:
		print(f"then: commit `chore: version {new}`, tag v{new}")
		return

	originals = {path: path.read_text() for path in changes}
	lock = ROOT / "Cargo.lock"
	originals[lock] = lock.read_text()
	try:
		for path, text in changes.items():
			path.write_text(text)
		run("cargo", "update", "--workspace", "--offline", capture=False)
		if wrong := [n for n, v, _ in packages() if v != new]:
			sys.exit(f"release: still not {new} after the edit: {', '.join(wrong)}")
		changed = run("git", "diff", "--name-only").split()
		if strays := sorted(set(changed) - {str(p.relative_to(ROOT)) for p in originals}):
			sys.exit(f"release: unexpected changes: {', '.join(strays)}")
		run("git", "add", "--", *changed)
		run("git", "commit", "-q", "-m", f"chore: version {new}")
	except BaseException:
		for path, text in originals.items():
			path.write_text(text)
		run("git", "reset", "-q")
		raise
	run("git", "tag", "-a", f"v{new}", "-m", f"bb {new}")
	commit = run("git", "rev-parse", "--short", "HEAD").strip()
	print(f"\ncommitted {commit} and tagged v{new}. To publish (starts the bundles workflow):")
	print(f"  git push origin {BRANCH}")
	print(f"  git push origin v{new}")


if __name__ == "__main__":
	main()
