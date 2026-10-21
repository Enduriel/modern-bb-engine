#!/usr/bin/env python3
"""test_all.py's runner stops what its binaries started, however it ends: a binary that
started a child of its own (window.rs starts gamescope and bb) leaves nothing running.

    python3 modern_bb_engine/scripts/test_all_stop_test.py
"""

import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "test_all.py"


def alive(pid):
	try:
		os.kill(pid, 0)
	except ProcessLookupError:
		return False
	# A zombie waiting for its reaper is gone too.
	try:
		return Path(f"/proc/{pid}/stat").read_text().split(") ")[1][0] != "Z"
	except OSError:
		return False


class StopTest(unittest.TestCase):
	def test_a_runner_ended_by_sigterm_leaves_no_grandchild(self):
		with tempfile.TemporaryDirectory() as tmp:
			pids = Path(tmp) / "pids"
			# A stand-in runner: test_all's own run_lanes and stop on one job, a shell
			# that starts a child (as window.rs starts gamescope) and waits.
			runner = f'''
import sys, time
sys.path.insert(0, {str(SCRIPT.parent)!r})
import test_all
job = test_all.Job("fake", "cpu", ["sh", "-c", "sleep 300 & echo $! > {pids}; wait"], ".", None, 0)
running = []
import signal
signal.signal(signal.SIGTERM, test_all.leave)
try:
	test_all.run_lanes([job], running, [], {{"cpu": 1}}, __import__("pathlib").Path({tmp!r}), time.monotonic())
finally:
	test_all.stop(running)
'''
			proc = subprocess.Popen([sys.executable, "-c", runner])
			deadline = time.monotonic() + 10
			while not (pids.exists() and pids.read_text().strip()) and time.monotonic() < deadline:
				time.sleep(0.05)
			grandchild = int(pids.read_text())
			self.assertTrue(alive(grandchild))
			proc.send_signal(signal.SIGTERM)
			proc.wait(timeout=20)
			time.sleep(0.2)
			if alive(grandchild):
				os.kill(grandchild, signal.SIGKILL)
				self.fail("the binary's child outlived the runner")


if __name__ == "__main__":
	unittest.main()
