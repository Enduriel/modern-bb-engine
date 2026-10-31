#!/usr/bin/env python3
"""refresh_mod_corpus.py against a Steam-like folder of its own: it finds the game through
Steam's library list and the storages in the Proton prefix, copies the mods and storages
(not the saves or a probe's) and pins them; it never writes to the game's
folders; the corpus stays as pinned while the installed mods change, until the next
refresh, which says what changed; it refuses during a probe run and inside a checkout.

	python3 scripts/refresh_mod_corpus_test.py
"""

import hashlib
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "refresh_mod_corpus.py"


def tree(root):
	"""{path: (bytes, mtime)} of every file under root."""
	return {str(p.relative_to(root)): (p.read_bytes(), p.stat().st_mtime_ns) for p in sorted(root.rglob("*")) if p.is_file()}


def pinned(pins):
	out = {}
	for line in pins.read_text().splitlines():
		if line and not line.startswith("#"):
			sha, size, rel = line.split(" ", 2)
			out[rel] = (int(size), sha)
	return out


def matches(corpus, pins):
	"""Whether the corpus folder holds exactly the pinned files."""
	found = {}
	for p in corpus.rglob("*"):
		if p.is_file() and p.parent != corpus:
			data = p.read_bytes()
			found[str(p.relative_to(corpus)).replace(os.sep, "/")] = (len(data), hashlib.sha256(data).hexdigest())
	return found == pinned(pins)


class Refresh(unittest.TestCase):
	def setUp(self):
		self.tmp = tempfile.TemporaryDirectory()
		root = Path(self.tmp.name)
		self.steam = root / "Steam"
		library = root / "library"
		(self.steam / "steamapps").mkdir(parents=True)
		(self.steam / "steamapps" / "libraryfolders.vdf").write_text(
			'"libraryfolders"\n{\n\t"0"\n\t{\n\t\t"path"\t\t"%s"\n\t}\n\t"1"\n\t{\n\t\t"path"\t\t"%s"\n\t}\n}\n' % (self.steam, library)
		)
		(library / "steamapps").mkdir(parents=True)
		(library / "steamapps" / "appmanifest_365360.acf").write_text('"AppState"\n{\n\t"installdir"\t\t"Battle Brothers"\n}\n')
		self.game = library / "steamapps" / "common" / "Battle Brothers"
		self.data = self.game / "data"
		self.data.mkdir(parents=True)
		for name, content in [("data_001.dat", "vanilla"), ("mod_a 1.0.zip", "a"), ("mod_b.zip", "b1")]:
			(self.data / name).write_text(content)
		self.documents = self.steam / "steamapps/compatdata/365360/pfx/drive_c/users/steamuser/Documents/Battle Brothers"
		saves = self.documents / "savegames"
		saves.mkdir(parents=True)
		for name, content in [("MSU#mod_msu#ModSettings.sav", "settings"), ("campaign.sav", "save"), ("bb_probe_MSU#x.sav", "probe")]:
			(saves / name).write_text(content)
		self.corpus = root / "mod-corpus"
		self.pins = root / "mod_corpus.txt"
		self.pins.write_text("# the header\n# kept\n")

	def tearDown(self):
		self.tmp.cleanup()

	def refresh(self, *extra):
		env = {k: v for k, v in os.environ.items() if k not in ("BB_DATA_DIR", "BB_MOD_CORPUS")}
		env["BB_STEAM_DIR"] = str(self.steam)
		cmd = [sys.executable, "-I", str(SCRIPT), "--to", str(self.corpus), "--pins", str(self.pins), *extra]
		return subprocess.run(cmd, env=env, capture_output=True, text=True)

	def test_it_copies_pins_and_moves_forward_only_when_run(self):
		before = tree(self.steam.parent / "library"), tree(self.steam)
		run = self.refresh()
		self.assertEqual(run.returncode, 0, run.stderr)
		self.assertEqual(sorted(pinned(self.pins)), ["mods/mod_a 1.0.zip", "mods/mod_b.zip", "storages/MSU#mod_msu#ModSettings.sav"])
		self.assertTrue(self.pins.read_text().startswith("# the header\n# kept\n"))
		self.assertTrue(matches(self.corpus, self.pins))
		self.assertTrue((self.corpus / "README.txt").is_file())
		# Only read: the game's folders are as they were, byte for byte and mtime.
		self.assertEqual((tree(self.steam.parent / "library"), tree(self.steam)), before)

		# The player updates and adds mods: the corpus and its pins don't move.
		(self.data / "mod_b.zip").write_text("b2")
		(self.data / "mod_c.zip").write_text("c")
		self.assertTrue(matches(self.corpus, self.pins))

		# A probe run holding the mods aside: refused, nothing changed.
		held = self.game / "_bb_probe_held_mods"
		held.mkdir()
		pins_before, corpus_before = self.pins.read_text(), tree(self.corpus)
		run = self.refresh()
		self.assertNotEqual(run.returncode, 0)
		self.assertIn("probe run", run.stderr)
		self.assertEqual((self.pins.read_text(), tree(self.corpus)), (pins_before, corpus_before))
		held.rmdir()
		(self.data / "mod_zz_bb_probe.zip").write_text("harness")
		self.assertIn("probe run", self.refresh().stderr)
		(self.data / "mod_zz_bb_probe.zip").unlink()

		# A dry run says what would change and changes nothing.
		run = self.refresh("--dry-run")
		self.assertEqual(run.returncode, 0, run.stderr)
		self.assertIn("changed: mods/mod_b.zip", run.stdout)
		self.assertEqual((self.pins.read_text(), tree(self.corpus)), (pins_before, corpus_before))

		# The deliberate refresh: the new set, the old one gone, what changed named.
		run = self.refresh()
		self.assertEqual(run.returncode, 0, run.stderr)
		self.assertIn("changed: mods/mod_b.zip", run.stdout)
		self.assertIn("added:   mods/mod_c.zip", run.stdout)
		self.assertIn("modded_main_menu.json", run.stdout)
		self.assertEqual((self.corpus / "mods" / "mod_b.zip").read_text(), "b2")
		self.assertTrue(matches(self.corpus, self.pins))
		self.assertEqual(sorted(p.name for p in self.corpus.parent.iterdir() if p.name.startswith("mod-corpus")), ["mod-corpus"])

	def test_a_corpus_inside_a_checkout_or_over_another_folder_is_refused(self):
		(self.corpus.parent / ".git").mkdir()
		run = self.refresh()
		self.assertNotEqual(run.returncode, 0)
		self.assertIn("git checkout", run.stderr)
		(self.corpus.parent / ".git").rmdir()
		self.corpus.mkdir()
		(self.corpus / "notes.txt").write_text("mine")
		run = self.refresh()
		self.assertNotEqual(run.returncode, 0)
		self.assertIn("more than a corpus", run.stderr)
		self.assertEqual((self.corpus / "notes.txt").read_text(), "mine")


if __name__ == "__main__":
	unittest.main()
