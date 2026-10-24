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

/// After a heap snapshot: the renderer gives back what taking it left in its allocator
/// (it took ~190 MB more of the renderer's C heap, measured 2026-10-24, which glibc kept
/// after it was freed), as at a scene's end (`UiHost::release_memory`).
pub fn release_after_heap_snapshot(game: &Game) {
	game.front().session().host().release_memory();
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
	let page = |name: &str| dump.page.iter().find(|(n, _)| n == name).map(|(_, v)| *v);
	part.line(format!(
		"the page: V8's JavaScript heap {} used of {}, Blink objects it holds {}, backing stores {}; DOM {} nodes in {} documents, {} listeners",
		page("js_heap.usedSize").map_or("?".into(), mib),
		page("js_heap.totalSize").map_or("?".into(), mib),
		page("js_heap.embedderHeapUsedSize").map_or("?".into(), mib),
		page("js_heap.backingStorageSize").map_or("?".into(), mib),
		page("dom.nodes").map_or("?".into(), |n| n.to_string()),
		page("dom.documents").map_or("?".into(), |n| n.to_string()),
		page("dom.jsEventListeners").map_or("?".into(), |n| n.to_string()),
	));
	part.line(format!("Chromium's processes (memory-infra, after a collection): {}", dump.processes.len()));
	let mut rows = Vec::new();
	for p in &dump.processes {
		let name = if p.name.is_empty() {
			crate::stats::process_role(p.pid as u32).map(|r| if r == "game" { "Browser (bb)".to_string() } else { r }).unwrap_or_default()
		} else {
			p.name.clone()
		};
		let footprint = total(p, "private_footprint_bytes");
		let top: Vec<&(String, u64)> = p.allocators.iter().filter(|(n, b)| !n.contains('/') && *b >= 1 << 19).collect();
		let parts: Vec<String> = top.iter().take(8).map(|(n, b)| format!("{n} {}", mib(*b))).collect();
		part.line(format!(
			"  {:>7} {:<22} private footprint {:>10}  ({})",
			p.pid,
			name,
			footprint.map_or("?".into(), mib),
			parts.join(", ")
		));
		for line in retained(p) {
			part.line(format!("            {line}"));
		}
		rows.push(
			obj()
				.with("pid", p.pid)
				.with("name", name.as_str())
				.with("totals", Json::Object(p.totals.iter().map(|(n, b)| (n.clone(), Json::from(*b))).collect()))
				.with("allocators", Json::Object(p.allocators.iter().map(|(n, b)| (n.clone(), Json::from(*b))).collect()))
				.with(
					"attributes",
					Json::Object(
						p.attributes
							.iter()
							.map(|(path, attrs)| (path.clone(), Json::Object(attrs.iter().map(|(n, v)| (n.clone(), Json::from(*v))).collect())))
							.collect(),
					),
				),
		);
	}
	part.json = obj()
		.with("file", "chromium-trace.json")
		.with("dumped", dump.dumped)
		.with("page", Json::Object(dump.page.iter().map(|(n, v)| (n.clone(), Json::from(*v))).collect()))
		.with("heap_snapshot", dump.heap_snapshot.as_ref().map(|(_, b)| *b))
		.with("processes", Json::Array(rows));
	part
}

/// What a process's allocators hold live against what they keep: Oilpan's committed pages
/// against its live objects, V8's heap, malloc's and PartitionAlloc's objects against
/// what they have from the system. Those that have memory only.
fn retained(p: &bb_ui::host::ChromiumProcess) -> Vec<String> {
	let mut lines = Vec::new();
	let f = |path: &str, name: &str| p.figure(path, name);
	if let (Some(committed), Some(live)) = (f("blink_gc/main/heap", "committed_size"), f("blink_gc/main/heap", "allocated_objects_size")) {
		lines.push(format!("Oilpan (Blink's heap): {} committed, {} of it live objects", mib(committed), mib(live)));
	}
	if let Some(heap) = f("v8/main/heap", "size") {
		lines.push(format!("V8's heaps: {} (all isolates' {})", mib(heap), f("v8", "size").map_or("?".into(), mib)));
	}
	if let Some(live) = f("malloc/allocated_objects", "size") {
		let mapped = f("malloc", "virtual_size").or(f("malloc", "size"));
		lines.push(format!("malloc: {} in use, {} from the system", mib(live), mapped.map_or("?".into(), mib)));
	}
	if let (Some(live), Some(committed)) = (f("partition_alloc/allocated_objects", "size"), f("partition_alloc", "size")) {
		lines.push(format!("PartitionAlloc: {} in use, {} committed", mib(live), mib(committed)));
	}
	lines
}
