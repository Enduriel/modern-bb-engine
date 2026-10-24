//! The snapshot's quick parts, each into a [`Part`]: the processes as the system counts
//! them, the GPU's memory (wgpu's allocator, the renderer's textures and pages), the
//! engine's services ([`squirrel_bb::memory_report`]) and the Squirrel VM's census.

use std::io::BufRead;
use std::path::Path;

use bb_render::Gpu;
use squirrel_bb::memory_report::{MemoryReport, ReportMemory};

use super::json::{Json, obj};
use super::{Part, mib};
use crate::game::Game;
use crate::stats::{self, ProcessMemory};

/// Every process of the game's: PSS and RSS by kind of memory, the GPU memory it
/// allocated, and the system's own figures.
pub fn processes() -> Part {
	let mut part = Part::new("processes");
	let Some(processes) = stats::tree_memory() else {
		part.error("not available on this system yet".to_string());
		return part;
	};
	let kinds = |kinds: &[(stats::MappingKind, u64)]| Json::Object(kinds.iter().map(|(k, b)| (k.name().to_string(), Json::from(*b))).collect());
	let figure = |p: &ProcessMemory, name: &str| p.figures.iter().find(|(n, _)| n == name).map(|(_, b)| *b);
	let rows = processes
		.iter()
		.map(|p| {
			obj()
				.with("pid", p.pid)
				.with("role", p.role.as_str())
				.with("pss", p.pss())
				.with("pss_by_kind", kinds(&p.kinds))
				.with("rss_by_kind", kinds(&p.rss_kinds))
				.with("vram", p.vram)
				.with("gtt", p.gtt)
				.with("figures", Json::Object(p.figures.iter().map(|(n, b)| (n.clone(), Json::from(*b))).collect()))
		})
		.collect();
	part.json = obj().with("processes", Json::Array(rows));
	let pss: u64 = processes.iter().map(ProcessMemory::pss).sum();
	let gpu: u64 = processes.iter().map(|p| p.vram + p.gtt).sum();
	if pss > 0 {
		part.line(format!("{} processes: {} PSS in all; GPU memory they allocated {}", processes.len(), mib(pss), mib(gpu)));
	} else {
		part.line(format!("{} processes", processes.len()));
	}
	for p in &processes {
		let mut line = format!("  {:>7} {:<24}", p.pid, p.role);
		// Chromium starts its tracing service for the first memory dump, and keeps it.
		if p.role.contains("TracingService") {
			line.push_str(" (started by a snapshot's Chromium dump)");
		}
		if p.pss() > 0 {
			line.push_str(&format!(" PSS {:>10}", mib(p.pss())));
			if let Some(rss) = figure(p, "VmRSS") {
				line.push_str(&format!("  RSS {:>10}", mib(rss)));
			}
			let parts: Vec<String> = p.kinds.iter().take(5).map(|(k, b)| format!("{} {}", k.name(), mib(*b))).collect();
			line.push_str(&format!("  ({})", parts.join(", ")));
			if p.vram + p.gtt > 0 {
				line.push_str(&format!("  VRAM {} GTT {}", mib(p.vram), mib(p.gtt)));
			}
		} else {
			let parts: Vec<String> = p.figures.iter().take(3).map(|(n, b)| format!("{n} {}", mib(*b))).collect();
			line.push_str(&parts.join(", "));
		}
		part.line(line);
	}
	part
}

/// wgpu's allocator, the renderer's textures, pages and glyph pages, the frame the game
/// draws into and the UI page's texture.
pub fn gpu(game: &Game, gpu: &Gpu, frame_bytes: u64) -> Part {
	let mut part = Part::new("gpu");
	let memory = game.renderer_memory();
	let residency = game.renderer_residency();
	part.line(format!("renderer: {memory}"));
	part.line(format!(
		"  the frame {}, glyph pages {} ({}), texture budget {}; frame {} now",
		mib(frame_bytes),
		residency.glyph_pages.0,
		mib(residency.glyph_pages.1),
		mib(residency.budget),
		residency.frame
	));
	let mut json = obj()
		.with("frame_now", residency.frame)
		.with("texture_budget", residency.budget)
		.with("frame_target", frame_bytes)
		.with("ui_page", memory.ui_page)
		.with("fog_mask", memory.fog_mask)
		.with("own", memory.own)
		.with("glyph_pages", obj().with("count", residency.glyph_pages.0).with("bytes", residency.glyph_pages.1))
		.with("loading", memory.loading)
		.with("failed", memory.failed)
		.with("warmed_waiting", obj().with("count", memory.held).with("bytes", memory.held_bytes));
	let textures = memory
		.textures
		.iter()
		.map(|t| {
			let drawn = residency.textures.iter().find(|d| d.path == t.path);
			obj()
				.with("path", t.path.as_str())
				.with("width", t.width)
				.with("height", t.height)
				.with("bytes", t.bytes)
				.with("stand_in", t.stand_in)
				.with("last_drawn_frame", drawn.map(|d| d.last_drawn))
				.with("drawn_by", drawn.map(|d| d.drawers.clone()))
		})
		.collect();
	json = json.with("textures", Json::Array(textures));
	let pages = residency
		.pages
		.iter()
		.map(|p| {
			obj()
				.with("page", p.page)
				.with("width", p.width)
				.with("height", p.height)
				.with("bytes", p.bytes)
				.with("frames", p.frames)
				.with("last_drawn_frame", p.last_drawn)
		})
		.collect();
	json = json.with("pages", Json::Array(pages));
	part.line(format!("  largest textures (of {}):", memory.textures.len()));
	for t in memory.textures.iter().take(12) {
		let ago = residency
			.textures
			.iter()
			.find(|d| d.path == t.path)
			.map_or("?".to_string(), |d| format!("{} frames ago by {}", residency.frame.saturating_sub(d.last_drawn), d.drawers));
		part.line(format!("    {:>10}  {}x{}  {}  last drawn {ago}", mib(t.bytes), t.width, t.height, t.path));
	}
	match gpu.device.generate_allocator_report() {
		Some(report) => {
			let mut by_name: std::collections::HashMap<&str, (u64, u64)> = std::collections::HashMap::new();
			for a in &report.allocations {
				let entry = by_name.entry(a.name.as_str()).or_default();
				entry.0 += 1;
				entry.1 += a.size;
			}
			let mut by_name: Vec<(&str, (u64, u64))> = by_name.into_iter().collect();
			by_name.sort_by(|a, b| b.1.1.cmp(&a.1.1).then_with(|| a.0.cmp(b.0)));
			part.line(format!(
				"wgpu's allocator: {} allocated in {} of blocks ({} blocks, {} allocations)",
				mib(report.total_allocated_bytes),
				mib(report.total_reserved_bytes),
				report.blocks.len(),
				report.allocations.len()
			));
			for (name, (count, bytes)) in by_name.iter().take(10) {
				part.line(format!("    {:>10}  {count:>5}x  {}", mib(*bytes), if name.is_empty() { "(unnamed)" } else { name }));
			}
			let allocator = obj()
				.with("allocated", report.total_allocated_bytes)
				.with("reserved", report.total_reserved_bytes)
				.with("blocks", Json::Array(report.blocks.iter().map(|b| obj().with("size", b.size).with("allocations", b.allocations.len())).collect()))
				.with(
					"by_name",
					Json::Array(by_name.iter().map(|(n, (c, b))| obj().with("name", *n).with("count", *c).with("bytes", *b)).collect()),
				);
			json = json.with("wgpu_allocator", allocator);
		}
		None => part.line("wgpu's allocator: this backend gives no report".to_string()),
	}
	part.json = json;
	part
}

/// What the engine's services say they hold, and the page's side where there is one.
pub fn engine(game: &Game) -> Part {
	let mut part = Part::new("engine");
	let mut report = MemoryReport::default();
	game.front().engine().report_memory(&mut report);
	game.front().report_memory(&mut report);
	part.line(format!("the engine's services: {} counted (estimates: their containers)", mib(report.bytes())));
	let mut owner = "";
	for item in &report.items {
		if item.owner != owner {
			owner = &item.owner;
			part.line(format!("  {owner}:"));
		}
		let bytes = if item.bytes > 0 { mib(item.bytes) } else { String::new() };
		part.line(format!("    {:<40} {:>9} {:>12}", item.what, item.count, bytes));
	}
	let rows = report
		.items
		.iter()
		.map(|i| obj().with("owner", i.owner.as_str()).with("what", i.what.as_str()).with("count", i.count).with("bytes", i.bytes))
		.collect();
	part.json = obj().with("items", Json::Array(rows)).with("in_battle", game.in_battle());
	part
}

/// The Squirrel VM's census (`census.txt`, which `parity/perf/script_memory.py` reads),
/// and its objects by kind.
pub fn census(game: &Game, dir: &Path) -> Part {
	let mut part = Part::new("census");
	let path = dir.join("census.txt");
	if let Err(e) = game.front().engine().vm().write_census(&path) {
		part.error(e);
		return part;
	}
	part.file("census.txt");
	match census_kinds(&path) {
		Ok(kinds) => {
			let (count, bytes) = kinds.iter().fold((0, 0), |(c, b), k| (c + k.1, b + k.2));
			part.line(format!("Squirrel VM: {count} objects, {} (the census's sizes, before malloc's)", mib(bytes)));
			let mut sorted = kinds.clone();
			sorted.sort_by_key(|k| std::cmp::Reverse(k.2));
			for (kind, count, bytes) in &sorted {
				part.line(format!("    {kind:<24} {count:>9} {:>12}", mib(*bytes)));
			}
			let rows = kinds.iter().map(|(k, c, b)| obj().with("kind", *k).with("count", *c).with("bytes", *b)).collect();
			part.json = obj().with("file", "census.txt").with("kinds", Json::Array(rows));
		}
		Err(e) => part.error(format!("reading the census back: {e}")),
	}
	part
}

/// The census's objects by kind: name, count, bytes (`T <ptr> <bytes> …` lines; `S`
/// gives the strings' count and bytes, `G` a function's debug info).
fn census_kinds(path: &Path) -> std::io::Result<Vec<(&'static str, u64, u64)>> {
	const KINDS: [(u8, &str); 10] = [
		(b'T', "tables"),
		(b'A', "arrays"),
		(b'C', "closures"),
		(b'N', "native closures"),
		(b'F', "function protos"),
		(b'O', "outers"),
		(b'U', "userdata"),
		(b'I', "instances"),
		(b'K', "classes"),
		(b'S', "strings"),
	];
	let mut totals = [(0u64, 0u64); 11];
	let reader = std::io::BufReader::new(std::fs::File::open(path)?);
	for line in reader.split(b'\n') {
		let line = line?;
		let Some(&kind) = line.first() else { continue };
		let field = |i: usize| line.split(|&b| b == b' ').nth(i).and_then(|f| std::str::from_utf8(f).ok()?.parse::<u64>().ok());
		if kind == b'S' {
			totals[9] = (field(1).unwrap_or(0), field(2).unwrap_or(0));
		} else if kind == b'G' {
			totals[10].1 += field(1).unwrap_or(0);
		} else if let Some(i) = KINDS.iter().position(|(k, _)| *k == kind) {
			totals[i].0 += 1;
			totals[i].1 += field(2).unwrap_or(0);
		}
	}
	let mut out: Vec<(&'static str, u64, u64)> = KINDS.iter().zip(totals).map(|((_, name), (c, b))| (*name, c, b)).collect();
	out.push(("functions' debug info", 0, totals[10].1));
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn counts_the_census_by_kind() {
		let dir = std::env::temp_dir().join(format!("bb-census-kinds-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("census.txt");
		std::fs::write(
			&path,
			"S 10 500 64 3 12 0\nZ table=88 node=32\nT 0x1 120 2 1 0x0 0\ns \"a\" 1\nT 0x2 88 0 0 0x0 0\nA 0x3 48 1 2 0\nF 0x4 200 1 1 1 1 1 1 1 1 \nG 30\n",
		)
		.unwrap();
		let kinds = census_kinds(&path).unwrap();
		let get = |name: &str| kinds.iter().find(|k| k.0 == name).map(|k| (k.1, k.2)).unwrap();
		assert_eq!(get("tables"), (2, 208));
		assert_eq!(get("arrays"), (1, 48));
		assert_eq!(get("strings"), (10, 500));
		assert_eq!(get("function protos"), (1, 200));
		assert_eq!(get("functions' debug info"), (0, 30));
		let _ = std::fs::remove_dir_all(&dir);
	}
}
