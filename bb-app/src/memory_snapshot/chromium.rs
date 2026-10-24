//! Chromium's part of the snapshot (the build with the UI): its memory-infra dump of the
//! page's processes, and with `--memory-snapshot-v8` V8's heap snapshot, asked through
//! CEF's own DevTools channel (`bb_ui::host::UiHost::start_memory_dump`). They take a
//! second or more, over the next frames.

use std::path::Path;
use std::time::Duration;

use bb_ui::host::ChromiumDump;

use super::json::{Json, obj};
use super::{Part, mib};
use crate::game::Game;

/// How long the dump may take before it's given up on: the dump alone, and with a heap
/// snapshot (100 MB and more, written as it comes).
pub const DUMP_TIME: Duration = Duration::from_secs(20);
pub const HEAP_SNAPSHOT_TIME: Duration = Duration::from_secs(180);

/// The heap snapshot's file name.
pub const HEAP_SNAPSHOT: &str = "v8.heapsnapshot";

/// Ask for the dump (and the heap snapshot into `dir`, with `v8`).
pub fn start(game: &Game, dir: &Path, v8: bool) -> Result<(), String> {
	game.front().session().host().start_memory_dump(v8.then(|| dir.join(HEAP_SNAPSHOT)))
}

/// The dump, once it's done; `None` while it's on its way.
pub fn poll(game: &Game) -> Option<ChromiumDump> {
	game.front().session().host().memory_dump_poll()
}

/// Give up on the dump: what came of it.
pub fn abandon(game: &Game, why: &str) -> Option<ChromiumDump> {
	game.front().session().host().abandon_memory_dump(why)
}

/// The part for `dump`, its trace written to `dir`.
pub fn part(dump: Option<ChromiumDump>, dir: &Path) -> Part {
	let mut part = Part::new("chromium");
	let Some(dump) = dump else {
		part.error("no dump came".to_string());
		return part;
	};
	part.took = dump.took;
	for e in &dump.errors {
		part.error(e.clone());
	}
	if dump.dumped != Some(true) {
		part.error(format!("Chromium's memory dump didn't succeed ({:?})", dump.dumped));
	}
	match std::fs::write(dir.join("chromium-trace.json"), &dump.trace) {
		Ok(()) => part.file("chromium-trace.json"),
		Err(e) => part.error(format!("chromium-trace.json: {e}")),
	}
	if let Some((path, bytes)) = &dump.heap_snapshot {
		part.file(HEAP_SNAPSHOT);
		part.line(format!("V8's heap snapshot of the page: {} ({}; open it in Chrome DevTools' Memory tab)", path.display(), mib(*bytes)));
	}
	let total = |p: &bb_ui::host::ChromiumProcess, name: &str| p.totals.iter().find(|(n, _)| n == name).map(|(_, b)| *b);
	part.line(format!("Chromium's processes (memory-infra, after a collection): {}", dump.processes.len()));
	let mut rows = Vec::new();
	for p in &dump.processes {
		let footprint = total(p, "private_footprint_bytes");
		let top: Vec<&(String, u64)> = p.allocators.iter().filter(|(n, b)| !n.contains('/') && *b >= 1 << 19).collect();
		let parts: Vec<String> = top.iter().take(8).map(|(n, b)| format!("{n} {}", mib(*b))).collect();
		part.line(format!(
			"  {:>7} {:<22} private footprint {:>10}  ({})",
			p.pid,
			p.name,
			footprint.map_or("?".into(), mib),
			parts.join(", ")
		));
		rows.push(
			obj()
				.with("pid", p.pid)
				.with("name", p.name.as_str())
				.with("totals", Json::Object(p.totals.iter().map(|(n, b)| (n.clone(), Json::from(*b))).collect()))
				.with("allocators", Json::Object(p.allocators.iter().map(|(n, b)| (n.clone(), Json::from(*b))).collect())),
		);
	}
	part.json = obj()
		.with("file", "chromium-trace.json")
		.with("dumped", dump.dumped)
		.with("heap_snapshot", dump.heap_snapshot.as_ref().map(|(_, b)| *b))
		.with("processes", Json::Array(rows));
	part
}
