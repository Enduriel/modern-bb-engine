//! A memory snapshot of the running game, for analysis outside it: asked for by the
//! hotkey Ctrl+Shift+F11 or, on Linux and macOS, the signal `SIGUSR1` ([`trigger`]), it
//! writes a folder `memory-<time>/` in bb's log folder (where log.html is), and log.html
//! says where. Nothing of it runs until asked.
//!
//! The folder:
//! - `summary.txt`: everything below in words, for a person;
//! - `snapshot.json`: the same as data, for tools (`parity/perf/memory_snapshot.py`
//!   summarizes one and compares two);
//! - the processes (bb and the page's Chromium processes): PSS and RSS by kind of memory,
//!   the GPU memory each allocated, the system's own figures ([`crate::stats`]);
//! - the C allocator: in use, free but held, per arena; glibc's `malloc-info.xml`
//!   ([`allocator`]);
//! - the GPU: wgpu's allocator, the renderer's textures (each with when it was last
//!   drawn), pages and glyph pages, the frame and the UI page's texture;
//! - the engine's services' records, caches and buffers ([`squirrel_bb::memory_report`]);
//! - the Squirrel VM's census (`census.txt`, for `parity/perf/script_memory.py`), by kind;
//! - with the UI, Chromium's own memory-infra dump of its processes
//!   (`chromium-trace.json`, which Perfetto opens) and, with `--memory-snapshot-v8`, V8's
//!   heap snapshot of the page (`v8.heapsnapshot`: 100 MB and more, and it holds the
//!   page's text).
//!
//! The quick parts take milliseconds, the census up to a second or so on a modded
//! campaign; they run at the frame the snapshot is asked at. Chromium's dump comes over
//! the next frames (its renderer's part runs on the page's main thread), and is given up
//! on after a while. A part that fails is noted and the others go on: nothing of it
//! ends or stops the game ([`crate::failure::catch`]).

mod allocator;
#[cfg(feature = "ui")]
mod chromium;
pub mod json;
mod parts;
pub mod trigger;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use bb_render::Gpu;
use squirrel_bb::html_log::{self, Class};

use self::json::{Json, obj};
use self::trigger::Trigger;
use crate::game::Game;

/// One part of a snapshot: its data, its lines in the summary, the files it wrote, and
/// what went wrong.
#[derive(Debug)]
pub struct Part {
	pub name: &'static str,
	pub took: Duration,
	pub json: Json,
	pub lines: Vec<String>,
	pub files: Vec<String>,
	pub errors: Vec<String>,
}

impl Part {
	pub fn new(name: &'static str) -> Part {
		Part { name, took: Duration::ZERO, json: Json::Null, lines: Vec::new(), files: Vec::new(), errors: Vec::new() }
	}

	pub fn line(&mut self, line: String) {
		self.lines.push(line);
	}

	pub fn file(&mut self, name: &str) {
		self.files.push(name.to_string());
	}

	pub fn error(&mut self, error: String) {
		self.errors.push(error);
	}
}

/// `bytes` in MiB, for a person.
pub fn mib(bytes: u64) -> String {
	format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

/// Run `collect` as the part `name`, timed; a panic in it is its error.
fn timed(name: &'static str, collect: impl FnOnce() -> Part) -> Part {
	let start = Instant::now();
	let mut part = crate::failure::catch(collect).unwrap_or_else(|e| {
		let mut part = Part::new(name);
		part.error(e);
		part
	});
	part.took = start.elapsed();
	part
}

/// A snapshot whose Chromium part is still on its way.
struct Pending {
	dir: PathBuf,
	trigger: Trigger,
	started: Instant,
	frame: u64,
	parts: Vec<Part>,
	/// When Chromium's part is given up on.
	#[cfg_attr(not(feature = "ui"), allow(dead_code))]
	deadline: Instant,
}

/// The snapshots: asked for, taken, written.
pub struct MemorySnapshots {
	/// bb's log folder, where the snapshots go (none: they can't be taken).
	logs: Option<PathBuf>,
	/// Whether Chromium's part takes V8's heap snapshot too (`--memory-snapshot-v8`).
	#[cfg_attr(not(feature = "ui"), allow(dead_code))]
	v8: bool,
	asked: Option<Trigger>,
	pending: Option<Pending>,
	/// The folder of the last snapshot finished, for tests and tools.
	last: Option<PathBuf>,
}

impl MemorySnapshots {
	pub fn new(logs: Option<PathBuf>, v8: bool) -> MemorySnapshots {
		MemorySnapshots { logs, v8, asked: None, pending: None, last: None }
	}

	/// A snapshot is asked for, by `trigger`: taken at the next frame ([`poll`](Self::poll)).
	pub fn ask(&mut self, trigger: Trigger) {
		self.asked.get_or_insert(trigger);
	}

	/// Whether one is being taken.
	pub fn busy(&self) -> bool {
		self.pending.is_some()
	}

	/// The folder of the last snapshot finished.
	pub fn last(&self) -> Option<&Path> {
		self.last.as_deref()
	}

	/// At a frame's end (`frame`, the game's count): take a snapshot asked for, and move
	/// one on its way on. `frame_bytes`: the frame the game draws into.
	pub fn poll(&mut self, game: &Game, gpu: &Gpu, frame_bytes: u64, frame: u64) {
		if trigger::take_signal() {
			self.ask(Trigger::Signal);
		}
		if let Some(pending) = self.pending.take() {
			self.pending = self.go_on(pending, game);
		}
		if let Some(trigger) = self.asked.take() {
			if self.pending.is_some() {
				html_log::engine(Class::Warning, "Memory", "A memory snapshot is still being written: the new one asked for is left out");
			} else {
				self.pending = self.begin(trigger, game, gpu, frame_bytes, frame);
			}
		}
	}

	/// Take the quick parts now, and ask for Chromium's; `None` once written.
	fn begin(&mut self, trigger: Trigger, game: &Game, gpu: &Gpu, frame_bytes: u64, frame: u64) -> Option<Pending> {
		let Some(logs) = &self.logs else {
			html_log::engine(Class::Warning, "Memory", "No memory snapshot: bb has no log folder to write it to");
			return None;
		};
		let started = Instant::now();
		let dir = logs.join(format!("memory-{}", bb_engine::log_files::stamp(SystemTime::now())));
		if let Err(e) = std::fs::create_dir_all(&dir) {
			html_log::engine(Class::Warning, "Memory", &format!("No memory snapshot: {}: {e}", dir.display()));
			return None;
		}
		html_log::engine(Class::Info, "Memory", &format!("Taking a memory snapshot (asked by {}) into {}", trigger.name(), dir.display()));
		// The processes first: before the snapshot's own work moves their figures.
		#[cfg_attr(not(feature = "ui"), allow(unused_mut))]
		let mut parts = vec![
			timed("processes", parts::processes),
			timed("allocator", || allocator::collect(&dir)),
			timed("gpu", || parts::gpu(game, gpu, frame_bytes)),
			timed("engine", || parts::engine(game)),
			timed("census", || parts::census(game, &dir)),
		];
		#[cfg(feature = "ui")]
		{
			let time = if self.v8 { chromium::HEAP_SNAPSHOT_TIME } else { chromium::DUMP_TIME };
			match crate::failure::catch(|| chromium::start(game, &dir, self.v8)) {
				Ok(Ok(())) => return Some(Pending { dir, trigger, started, frame, parts, deadline: Instant::now() + time }),
				Ok(Err(e)) | Err(e) => {
					let mut part = Part::new("chromium");
					part.error(e);
					parts.push(part);
				}
			}
		}
		let pending = Pending { dir, trigger, started, frame, parts, deadline: Instant::now() };
		self.finish(pending, game);
		None
	}

	/// Chromium's part on its way: written once it came or its time is up.
	#[cfg(feature = "ui")]
	fn go_on(&mut self, mut pending: Pending, game: &Game) -> Option<Pending> {
		let dump = match crate::failure::catch(|| chromium::poll(game)) {
			Ok(Some(dump)) => Some(dump),
			Ok(None) if Instant::now() < pending.deadline => return Some(pending),
			Ok(None) => crate::failure::catch(|| chromium::abandon(game, "it took too long")).ok().flatten(),
			Err(e) => {
				let mut part = Part::new("chromium");
				part.error(e);
				pending.parts.push(part);
				self.finish(pending, game);
				return None;
			}
		};
		let dir = pending.dir.clone();
		// Its time is the dump's, from asking to its last answer.
		let took = dump.as_ref().map(|d| d.took);
		if dump.as_ref().is_some_and(|d| d.heap_snapshot.is_some()) {
			let _ = crate::failure::catch(|| chromium::release_after_heap_snapshot(game));
		}
		let mut part = timed("chromium", || chromium::part(dump, &dir));
		part.took = took.unwrap_or(part.took);
		pending.parts.push(part);
		self.finish(pending, game);
		None
	}

	#[cfg(not(feature = "ui"))]
	fn go_on(&mut self, pending: Pending, game: &Game) -> Option<Pending> {
		self.finish(pending, game);
		None
	}

	/// Write the summary and the JSON, and say where in the log.
	fn finish(&mut self, pending: Pending, game: &Game) {
		let Pending { dir, trigger, started, frame, parts, .. } = pending;
		let took = started.elapsed();
		let stamp = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
		let failed: Vec<String> = parts.iter().filter(|p| !p.errors.is_empty()).map(|p| format!("{}: {}", p.name, p.errors.join("; "))).collect();
		let mut files = vec!["summary.txt".to_string(), "snapshot.json".to_string()];
		files.extend(parts.iter().flat_map(|p| p.files.iter().cloned()));
		// The summary.
		let mut text = format!("Memory snapshot {stamp}\n{}\n", crate::version());
		text += &format!(
			"Asked by {} at frame {frame}, {}; written in {:.0} ms\n",
			trigger.name(),
			if game.in_battle() { "in a battle" } else { "not in a battle" },
			took.as_secs_f64() * 1000.0
		);
		let times: Vec<String> = parts.iter().map(|p| format!("{} {:.1} ms{}", p.name, p.took.as_secs_f64() * 1000.0, if p.errors.is_empty() { "" } else { " (failed)" })).collect();
		text += &format!("Parts: {}\n", times.join(", "));
		for p in &parts {
			text += &format!("\n== {} ==\n", p.name);
			for line in &p.lines {
				text += line;
				text.push('\n');
			}
			for e in &p.errors {
				text += &format!("FAILED: {e}\n");
			}
		}
		text += &format!("\nFiles: {}\n", files.join(", "));
		// The data.
		let part_rows = parts
			.iter()
			.map(|p| {
				obj()
					.with("name", p.name)
					.with("ok", p.errors.is_empty())
					.with("ms", p.took.as_secs_f64() * 1000.0)
					.with("errors", Json::Array(p.errors.iter().map(|e| Json::from(e.as_str())).collect()))
					.with("files", Json::Array(p.files.iter().map(|f| Json::from(f.as_str())).collect()))
			})
			.collect();
		let mut json = obj()
			.with("format", 1u64)
			.with("time", stamp.trim_start_matches("memory-"))
			.with("version", crate::version())
			.with("trigger", match trigger {
				Trigger::Hotkey => "hotkey",
				Trigger::Signal => "signal",
			})
			.with("frame", frame)
			.with("in_battle", game.in_battle())
			.with("ms", took.as_secs_f64() * 1000.0)
			.with("parts", Json::Array(part_rows));
		for p in parts {
			json = json.with(p.name, p.json);
		}
		let written = std::fs::write(dir.join("summary.txt"), text).and_then(|()| std::fs::write(dir.join("snapshot.json"), json.to_text()));
		match written {
			Ok(()) => {
				let mut line = format!("Memory snapshot written to {} in {:.0} ms (summary.txt)", dir.display(), took.as_secs_f64() * 1000.0);
				if !failed.is_empty() {
					line += &format!("; parts that failed: {}", failed.join(" | "));
				}
				html_log::engine(if failed.is_empty() { Class::Info } else { Class::Warning }, "Memory", &line);
				self.last = Some(dir);
			}
			Err(e) => html_log::engine(Class::Warning, "Memory", &format!("The memory snapshot in {} couldn't be written: {e}", dir.display())),
		}
	}
}
