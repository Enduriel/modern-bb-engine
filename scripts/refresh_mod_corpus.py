#!/usr/bin/env python3
"""Move the reference mod corpus forward: copy the installed mods and their storages into
the corpus folder (replacing the set there), rewrite the pin list and say which goldens
to retake.

The mod corpus tests (`bb_engine::mod_corpus`) never mount the installed mods: they mount
a reference copy kept outside the repository (the mods are their authors', the repository
is public), pinned file by file by size and SHA-256 in `bb-engine/src/mod_corpus.txt`.
Updating the installed mods changes nothing for them until this script runs.

	scripts/refresh_mod_corpus.py [--to DIR] [--data DIR] [--documents DIR] [--dry-run]

- The corpus folder: `--to`, else `BB_MOD_CORPUS`, else the first folder named
  `mod-corpus` beside the checkout or beside a folder above it (as the tests find it).
  The first time there is none: name it with `--to`, outside any git checkout.
- The installed mods: every `.zip` in the game's data folder: `--data` (the data folder
  or the game's), else `BB_DATA_DIR`, else the game in Steam's libraries (`BB_STEAM_DIR`
  names Steam's folder, else the usual places), as bb finds it.
- Their storages: every `<mod>#<name>.sav` in the original's savegames folder, in its
  documents folder: `--documents`, else (Linux) the Proton prefix's `Documents/Battle
  Brothers` under Steam's folder, or (Windows) `Documents/Battle Brothers`.

The game's folders are only read, never written. While a probe run (which holds the
installed mods aside) is going on, it refuses: try again once the run has ended.
`--dry-run` says what would change without copying or rewriting anything.
"""

import argparse
import hashlib
import os
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PINS = ROOT / "bb-engine" / "src" / "mod_corpus.txt"
FOLDER = "mod-corpus"
VAR = "BB_MOD_CORPUS"
MODS, STORAGES = "mods", "storages"
APP_ID = "365360"
# What a probe run leaves in the game's folders while it holds the installed mods and
# storages aside (bb_engine::probe_aside, and the runner's own names).
PROBE_RUN_SIGNS_GAME = ["_bb_probe_held_mods", "_bb_probe_with_mods.json"]
PROBE_RUN_SIGNS_DOCUMENTS = ["_bb_probe_held_settings", "_bb_probe_held_settings.json", "savegames.bb_probe_backup"]
HARNESS_MOD = "mod_zz_bb_probe"
README = """The reference mod corpus of the modern Battle Brothers engine's mod corpus tests.

mods/      the mod archives they mount (instead of the installed mods)
storages/  the mods' storages (<mod>#<name>.sav) they start from

Pinned by size and SHA-256 in bb-engine/src/mod_corpus.txt of the code repository; a
changed file fails the tests. Made by scripts/refresh_mod_corpus.py: run it again to move
the corpus forward, never edit this folder by hand. The mods are their authors': keep this
folder out of any repository.
"""


def fail(message):
	sys.exit(f"refresh_mod_corpus: {message}")


def sha256(path):
	h = hashlib.sha256()
	with open(path, "rb") as f:
		for chunk in iter(lambda: f.read(1 << 20), b""):
			h.update(chunk)
	return h.hexdigest()


def env_dir(var):
	value = os.environ.get(var)
	return Path(value) if value else None


# --- where things are ---------------------------------------------------------------

def corpus_dir(given):
	if given:
		return Path(given).expanduser().resolve()
	if env_dir(VAR):
		return env_dir(VAR).expanduser().resolve()
	for folder in [ROOT, *ROOT.parents]:
		if (folder / FOLDER).is_dir():
			return folder / FOLDER
	fail(f"no corpus folder yet: {VAR} is unset and no folder named {FOLDER} is beside {ROOT} or a folder "
		 f"above it. Name one with --to DIR (outside any git checkout; call it {FOLDER} and put it beside the "
		 f"checkout, or set {VAR}=DIR, so the tests find it)")


def steam_dirs():
	"""Steam's folders: BB_STEAM_DIR's alone, else the usual places for this system."""
	if env_dir("BB_STEAM_DIR"):
		return [env_dir("BB_STEAM_DIR")]
	home = Path.home()
	if sys.platform == "win32":
		found = []
		try:
			import winreg
			with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Valve\Steam") as key:
				found.append(Path(winreg.QueryValueEx(key, "SteamPath")[0]))
		except OSError:
			pass
		found.append(Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Steam")
		candidates = found
	elif sys.platform == "darwin":
		candidates = [home / "Library/Application Support/Steam"]
	else:
		candidates = [
			home / ".local/share/Steam",
			home / ".steam/steam",
			home / ".steam/root",
			home / ".var/app/com.valvesoftware.Steam/.local/share/Steam",
			home / "snap/steam/common/.local/share/Steam",
		]
	seen, dirs = set(), []
	for d in candidates:
		if (d / "steamapps").is_dir() and d.resolve() not in seen:
			seen.add(d.resolve())
			dirs.append(d)
	return dirs


def vdf_values(text, key):
	return [m.group(1).replace("\\\\", "\\") for m in re.finditer(r'"%s"\s+"((?:[^"\\]|\\.)*)"' % re.escape(key), text, re.I)]


def libraries(steam):
	libs = [steam]
	vdf = steam / "steamapps" / "libraryfolders.vdf"
	if vdf.is_file():
		libs += [Path(p) for p in vdf_values(vdf.read_text(errors="replace"), "path")]
	return libs


def game_in_steam():
	"""(the game's data folder, Steam's folder, its library) in Steam's libraries, or None."""
	for steam in steam_dirs():
		for library in libraries(steam):
			manifest = library / "steamapps" / f"appmanifest_{APP_ID}.acf"
			if not manifest.is_file():
				continue
			for name in vdf_values(manifest.read_text(errors="replace"), "installdir") or ["Battle Brothers"]:
				data = library / "steamapps" / "common" / name / "data"
				if (data / "data_001.dat").is_file():
					return data, steam, library
	return None


def data_dir_in(folder):
	for d in (folder / "data", folder):
		if (d / "data_001.dat").is_file():
			return d
	return None


def find_data(given):
	folder = Path(given).expanduser() if given else env_dir("BB_DATA_DIR")
	if folder:
		data = data_dir_in(folder)
		if data is None:
			fail(f"{folder} isn't the game's folder (no data/data_001.dat in it)")
		return data, []
	found = game_in_steam()
	if found is None:
		fail("the game isn't in Steam's libraries: name its folder with --data (or BB_DATA_DIR)")
	data, steam, library = found
	return data, [steam, library]


def find_documents(given, steam_places):
	if given:
		return Path(given).expanduser()
	if sys.platform == "win32":
		return Path(os.environ.get("USERPROFILE", Path.home())) / "Documents" / "Battle Brothers"
	places = steam_places or steam_dirs()
	for place in places:
		docs = place / "steamapps/compatdata" / APP_ID / "pfx/drive_c/users/steamuser/Documents/Battle Brothers"
		if docs.is_dir():
			return docs
	fail("the original's documents folder (Documents/Battle Brothers, with its savegames) wasn't found: "
		 "name it with --documents")


# --- the copy -----------------------------------------------------------------------

def inside_git_checkout(path):
	return any((p / ".git").exists() for p in [path, *path.parents])


def check_no_probe_run(data, documents):
	game = data.parent
	signs = [game / s for s in PROBE_RUN_SIGNS_GAME] + [documents / s for s in PROBE_RUN_SIGNS_DOCUMENTS]
	signs += sorted(data.glob(HARNESS_MOD + "*.zip"))
	present = [str(s) for s in signs if s.exists()]
	if present:
		fail("a probe run holds the installed mods or storages aside (" + ", ".join(present) +
			 "): try again once it has ended")


def installed(data, documents):
	"""{corpus path: source file}: the installed mods and the storages."""
	files = {}
	for zip_ in sorted(data.glob("*.zip")):
		if zip_.is_file() and not zip_.name.startswith(HARNESS_MOD):
			files[f"{MODS}/{zip_.name}"] = zip_
	saves = documents / "savegames"
	if not saves.is_dir():
		fail(f"{saves} isn't a folder: is {documents} the original's documents folder?")
	for sav in sorted(saves.glob("*.sav")):
		if "#" in sav.name and not sav.name.startswith("bb_probe_") and sav.is_file():
			files[f"{STORAGES}/{sav.name}"] = sav
	return files


def copy_into(staging, files):
	"""Copies each file (read only) into the staging folder; {path: (size, sha256)}."""
	pins = {}
	for rel, source in files.items():
		dest = staging / rel
		dest.parent.mkdir(parents=True, exist_ok=True)
		h, size = hashlib.sha256(), 0
		with open(source, "rb") as f, open(dest, "wb") as out:
			for chunk in iter(lambda: f.read(1 << 20), b""):
				h.update(chunk)
				out.write(chunk)
				size += len(chunk)
		shutil.copystat(source, dest)
		# Changed while it was read (the game or Steam writing it): say so, don't pin it.
		if sha256(source) != h.hexdigest():
			fail(f"{source} changed while it was copied: try again")
		pins[rel] = (size, h.hexdigest())
	return pins


def replace_corpus(corpus, staging):
	known = {MODS, STORAGES, "README.txt"}
	if corpus.exists():
		other = sorted(p.name for p in corpus.iterdir() if p.name not in known)
		if other:
			fail(f"{corpus} holds more than a corpus ({', '.join(other)}): not replacing it")
		old = corpus.with_name(corpus.name + f".old-{os.getpid()}")
		corpus.rename(old)
		staging.rename(corpus)
		shutil.rmtree(old)
	else:
		staging.rename(corpus)


# --- the pin list -------------------------------------------------------------------

def read_pins(text):
	pins = {}
	for line in text.splitlines():
		if not line.strip() or line.startswith("#"):
			continue
		parts = line.split(" ", 2)
		if len(parts) == 3 and len(parts[0]) == 64:
			pins[parts[2]] = (int(parts[1]), parts[0])
	return pins


def write_pins(path, old_text, pins):
	header = []
	for line in old_text.splitlines():
		if not line.startswith("#"):
			break
		header.append(line)
	lines = header + [f"{sha} {size} {rel}" for rel, (size, sha) in sorted(pins.items())]
	path.write_text("\n".join(lines) + "\n")


def changes(old, new):
	out = []
	for rel in sorted(set(old) | set(new)):
		if rel not in new:
			out.append(f"  removed: {rel}")
		elif rel not in old:
			out.append(f"  added:   {rel}")
		elif old[rel][1] != new[rel][1]:
			out.append(f"  changed: {rel}")
	return out


GOLDENS = """
Retake the goldens taken with the corpus, now, while the installed mods and their storages
are still the ones copied (the original game runs with the installed ones):

- bb-ui/tests/goldens/modded_main_menu.json and modded_mod_options.json: the original's
  UI with these mods and storages (bb-ui/probes/README.md, `modded_goldens.js`). With
  the probe runner (BB_PROBE_RUNNER):
      cat bb-ui/src/parity/snapshot.js bb-ui/probes/modded_goldens.js > OUT/modded.js
      "$BB_PROBE_RUNNER" --js OUT/modded.js --mode menu --keep-mods --out OUT
  then copy OUT/js_*_golden_modded_main_menu.json to bb-ui/tests/goldens/modded_main_menu.json
  and OUT/js_*_golden_modded_mod_options.json to bb-ui/tests/goldens/modded_mod_options.json.
  Check that every difference from the old goldens comes from the mods or storages that
  changed before keeping them, and note the retake in bb-ui/probes/README.md.

Then run the corpus tests:
    cargo test --workspace --profile test-release --features bb-ui/cef,bb-app/ui \\
        --test mod_frameworks --test smoke --test archive_index --test world_size_settings \\
        --test mods_ui --test mods_battle_ui --test mods_start_input --test window
"""


def main():
	ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
	ap.add_argument("--to", help=f"the corpus folder (default: {VAR}, else the {FOLDER} folder the tests find)")
	ap.add_argument("--data", help="the game's data folder or the game's folder (default: BB_DATA_DIR, else Steam's)")
	ap.add_argument("--documents", help="the original's Documents/Battle Brothers folder")
	ap.add_argument("--dry-run", action="store_true", help="say what would change; copy and rewrite nothing")
	ap.add_argument("--pins", type=Path, default=PINS, help="the pin list to rewrite (default: %(default)s)")
	args = ap.parse_args()

	corpus = corpus_dir(args.to)
	if inside_git_checkout(corpus):
		fail(f"{corpus} is inside a git checkout: the corpus must stay out of the repositories")
	data, steam_places = find_data(args.data)
	documents = find_documents(args.documents, steam_places)
	check_no_probe_run(data, documents)
	files = installed(data, documents)
	if not any(rel.startswith(MODS + "/") for rel in files):
		fail(f"no mods installed in {data}")

	old_text = args.pins.read_text() if args.pins.is_file() else ""
	old = read_pins(old_text)
	print(f"mods from     {data}")
	print(f"storages from {documents / 'savegames'}")
	print(f"corpus        {corpus}")
	if args.dry_run:
		new = {rel: (src.stat().st_size, sha256(src)) for rel, src in files.items()}
		print("\n".join(changes(old, new)) or "  no changes")
		print("(dry run: nothing copied or rewritten)")
		return
	corpus.parent.mkdir(parents=True, exist_ok=True)
	staging = corpus.with_name(corpus.name + f".new-{os.getpid()}")
	if staging.exists():
		shutil.rmtree(staging)
	staging.mkdir()
	try:
		new = copy_into(staging, files)
		(staging / "README.txt").write_text(README)
		# The installed files again: none came or went while copying.
		check_no_probe_run(data, documents)
		if set(installed(data, documents)) != set(files):
			fail("the installed mods or storages changed while they were copied: try again")
		replace_corpus(corpus, staging)
	finally:
		if staging.exists():
			shutil.rmtree(staging)
	write_pins(args.pins, old_text, new)
	mods = sum(1 for rel in new if rel.startswith(MODS + "/"))
	print(f"copied {mods} mods and {len(new) - mods} storages; pinned them in {args.pins}")
	diff = changes(old, new)
	print("changes from the old pins:\n" + ("\n".join(diff) if diff else "  none"))
	if diff:
		print(GOLDENS)


if __name__ == "__main__":
	main()
