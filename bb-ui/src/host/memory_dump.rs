//! Chromium's own breakdown of the page's memory, for the game's memory snapshot
//! (`bb_app::memory_snapshot`): a detailed memory-infra dump of every process of the
//! page's (what `parity/perf/cdp_memory.py` reads over a DevTools port), and optionally
//! V8's heap snapshot of the page, both asked through CEF's own DevTools channel
//! (`SendDevToolsMessage` and a message observer), so they need no `--devtools` port.
//!
//! The calls go one after the other, each sent from the answer to the last, on CEF's UI
//! thread: `Tracing.start` (the memory-infra category only, no periodic dumps),
//! `Tracing.requestMemoryDump` (detailed, after a collection), `Tracing.end`; the trace
//! comes back as `Tracing.dataCollected` events, then `Tracing.tracingComplete`. Then the
//! page's JavaScript heap (`Runtime.getHeapUsage`: V8's used and total, and Blink's
//! objects V8 holds) and its DOM's counts (`Memory.getDOMCounters`: documents, nodes,
//! listeners), which tell live data growing from memory kept after it's freed. With a
//! heap snapshot asked for, then `HeapProfiler.takeHeapSnapshot`, whose chunks are
//! written to its file as they come. Nothing of this exists until a dump is asked for.
//!
//! The renderer's parts (V8's, Blink's) run on its main thread, so the page must get its
//! turns meanwhile: the game wakes it every frame while a dump is on its way
//! ([`UiHost::memory_dump_poll`](super::UiHost::memory_dump_poll)). The game gives up on
//! a dump that takes too long ([`Dumps::abandon`]): tracing still on is ended then.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cef::*;

use super::client::Shared;
use crate::json::Json;

/// The DevTools message ids the dumps' calls take: their own range, apart from the
/// memory releases' (`memory::NEXT_MESSAGE`).
const FIRST_ID: i32 = 1 << 26;

/// Where a dump is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
	/// `Tracing.start` sent.
	Starting,
	/// `Tracing.requestMemoryDump` sent.
	Dumping,
	/// `Tracing.end` sent: the trace is coming.
	Collecting,
	/// `Runtime.getHeapUsage` sent.
	HeapUsage,
	/// `Memory.getDOMCounters` sent.
	DomCounters,
	/// `HeapProfiler.takeHeapSnapshot` sent: its chunks are coming.
	HeapSnapshot,
	Done,
}

/// A dump on its way, as the observer fills it.
struct Flight {
	sent: Instant,
	step: Step,
	/// The id of the call whose answer moves it on.
	waiting: i32,
	next_id: i32,
	/// `Tracing.dataCollected`'s params, as received.
	collected: Vec<Vec<u8>>,
	/// Whether the memory dump said it succeeded.
	dumped: Option<bool>,
	/// The page's figures (`js_heap.usedSize`, `dom.nodes`, …).
	page: Vec<(String, u64)>,
	/// The heap snapshot's file, while its chunks come, and its bytes so far.
	heap: Option<(PathBuf, Option<std::fs::File>, u64)>,
	errors: Vec<String>,
	/// The calls to send once the lock is let go (an answer can come within the send).
	outbox: Vec<Vec<u8>>,
}

/// The dump on its way, if any: shared with the observer on CEF's UI thread.
#[derive(Default)]
pub(super) struct Dumps(Mutex<Option<Flight>>);

/// A finished (or abandoned) dump.
#[derive(Debug, Default)]
pub struct ChromiumDump {
	/// The trace's events, as a Chrome JSON trace (`{"traceEvents": [...]}`: Perfetto and
	/// `chrome://tracing` open it).
	pub trace: String,
	/// Each process's figures from it.
	pub processes: Vec<super::memory_dump_trace::ChromiumProcess>,
	/// Whether Chromium said the memory dump succeeded (`None`: it never answered).
	pub dumped: Option<bool>,
	/// The page's own figures: its JavaScript heap (`js_heap.usedSize`, `js_heap.totalSize`,
	/// `js_heap.embedderHeapUsedSize`, `js_heap.backingStorageSize`) and its DOM's counts
	/// (`dom.documents`, `dom.nodes`, `dom.jsEventListeners`).
	pub page: Vec<(String, u64)>,
	/// The V8 heap snapshot's file and bytes, if one was asked for and written.
	pub heap_snapshot: Option<(PathBuf, u64)>,
	/// What went wrong (a call refused, a timeout).
	pub errors: Vec<String>,
	pub took: Duration,
}

impl Dumps {
	fn state(&self) -> std::sync::MutexGuard<'_, Option<Flight>> {
		self.0.lock().unwrap_or_else(|e| e.into_inner())
	}

	/// Whether a dump is on its way.
	pub fn in_flight(&self) -> bool {
		self.state().as_ref().is_some_and(|f| f.step != Step::Done)
	}

	/// Start a dump through `host`, writing a heap snapshot to `heap` after it if given.
	/// `false` if one is already on its way.
	pub fn start(&self, host: &BrowserHost, heap: Option<PathBuf>) -> bool {
		let mut state = self.state();
		if state.as_ref().is_some_and(|f| f.step != Step::Done) {
			return false;
		}
		let mut flight = Flight {
			sent: Instant::now(),
			step: Step::Starting,
			waiting: 0,
			next_id: FIRST_ID,
			collected: Vec::new(),
			dumped: None,
			page: Vec::new(),
			heap: heap.map(|path| (path, None, 0)),
			errors: Vec::new(),
			outbox: Vec::new(),
		};
		let config = Json::Object(vec![
			(
				"traceConfig".into(),
				Json::Object(vec![
					("includedCategories".into(), Json::Array(vec![Json::Str("disabled-by-default-memory-infra".into())])),
					("memoryDumpConfig".into(), Json::Object(vec![("triggers".into(), Json::Array(Vec::new()))])),
				]),
			),
			("transferMode".into(), Json::Str("ReportEvents".into())),
		]);
		flight.waiting = flight.send("Tracing.start", config);
		let outbox = std::mem::take(&mut flight.outbox);
		*state = Some(flight);
		drop(state);
		send_all(host, outbox);
		true
	}

	/// The dump, once done (taken: the next call returns `None` until another is).
	pub fn take_done(&self) -> Option<ChromiumDump> {
		let mut state = self.state();
		if state.as_ref().is_none_or(|f| f.step != Step::Done) {
			return None;
		}
		state.take().map(Flight::finish)
	}

	/// Give up on the dump on its way: what came so far, with `why` among its errors.
	/// Tracing still on is ended through `host`, and what comes after is dropped.
	pub fn abandon(&self, host: Option<&BrowserHost>, why: &str) -> Option<ChromiumDump> {
		let mut flight = self.state().take()?;
		let step = flight.step;
		flight.errors.push(format!("{why} (at {step:?})"));
		if let (Some(host), Step::Starting | Step::Dumping) = (host, step) {
			flight.send("Tracing.end", Json::Object(Vec::new()));
			send_all(host, std::mem::take(&mut flight.outbox));
		}
		Some(flight.finish())
	}

	/// A DevTools call answered.
	fn answered(&self, host: Option<&BrowserHost>, id: i32, success: bool, result: &[u8]) {
		let outbox = self.answered_locked(host.is_some(), id, success, result);
		if let Some(host) = host {
			send_all(host, outbox);
		}
	}

	/// [`answered`](Self::answered) under the lock: the calls to send next.
	fn answered_locked(&self, has_host: bool, id: i32, success: bool, result: &[u8]) -> Vec<Vec<u8>> {
		let mut state = self.state();
		let Some(flight) = state.as_mut() else { return Vec::new() };
		if id != flight.waiting || flight.step == Step::Done {
			return Vec::new();
		}
		let text = String::from_utf8_lossy(result);
		if !success {
			flight.errors.push(format!("{:?} refused: {text}", flight.step));
		}
		if !has_host {
			flight.errors.push("no browser to send the next call to".into());
			flight.step = Step::Done;
			return Vec::new();
		}
		match (flight.step, success) {
			(Step::Starting, true) => {
				let params = Json::Object(vec![
					("deterministic".into(), Json::Bool(true)),
					("levelOfDetail".into(), Json::Str("detailed".into())),
				]);
				flight.waiting = flight.send("Tracing.requestMemoryDump", params);
				flight.step = Step::Dumping;
			}
			(Step::Starting, false) => flight.step = Step::Done,
			(Step::Dumping, _) => {
				let ok = success && Json::parse(&text).ok().and_then(|j| j.get("success").cloned()) == Some(Json::Bool(true));
				flight.dumped = Some(ok);
				// Ends tracing either way; the trace comes as events.
				flight.send("Tracing.end", Json::Object(Vec::new()));
				flight.waiting = 0;
				flight.step = Step::Collecting;
			}
			(Step::HeapUsage, _) => {
				flight.page.extend(numbers("js_heap.", &text));
				flight.waiting = flight.send("Memory.getDOMCounters", Json::Object(Vec::new()));
				flight.step = Step::DomCounters;
			}
			(Step::DomCounters, _) => {
				flight.page.extend(numbers("dom.", &text));
				flight.heap_snapshot_or_done();
			}
			(Step::HeapSnapshot, _) => {
				if let Some((_, file, _)) = &mut flight.heap
					&& let Some(mut f) = file.take()
					&& let Err(e) = f.flush()
				{
					flight.errors.push(format!("the heap snapshot's file: {e}"));
				}
				flight.send("HeapProfiler.disable", Json::Object(Vec::new()));
				flight.step = Step::Done;
			}
			_ => {}
		}
		std::mem::take(&mut flight.outbox)
	}

	/// A DevTools event came.
	fn event(&self, host: Option<&BrowserHost>, method: &str, params: &[u8]) {
		let outbox = self.event_locked(host.is_some(), method, params);
		if let Some(host) = host {
			send_all(host, outbox);
		}
	}

	/// [`event`](Self::event) under the lock: the calls to send next.
	fn event_locked(&self, has_host: bool, method: &str, params: &[u8]) -> Vec<Vec<u8>> {
		let mut state = self.state();
		let Some(flight) = state.as_mut() else { return Vec::new() };
		match (method, flight.step) {
			("Tracing.dataCollected", Step::Collecting | Step::Dumping) => flight.collected.push(params.to_vec()),
			("Tracing.tracingComplete", Step::Collecting) if has_host => {
				flight.waiting = flight.send("Runtime.getHeapUsage", Json::Object(Vec::new()));
				flight.step = Step::HeapUsage;
			}
			("Tracing.tracingComplete", Step::Collecting) => flight.step = Step::Done,
			("HeapProfiler.addHeapSnapshotChunk", Step::HeapSnapshot) => {
				let chunk = Json::parse(&String::from_utf8_lossy(params)).ok();
				let chunk = chunk.as_ref().and_then(|j| j.get("chunk")).and_then(Json::as_str);
				if let (Some(chunk), Some((_, Some(file), bytes))) = (chunk, &mut flight.heap) {
					match file.write_all(chunk.as_bytes()) {
						Ok(()) => *bytes += chunk.len() as u64,
						Err(e) => {
							flight.errors.push(format!("the heap snapshot's file: {e}"));
							flight.heap.as_mut().map(|h| h.1.take());
						}
					}
				}
			}
			_ => {}
		}
		std::mem::take(&mut flight.outbox)
	}
}

/// Send `messages` through `host`. Off CEF's UI thread each is posted there (and the
/// call returns 0); on it, an answer can come before the call returns.
fn send_all(host: &BrowserHost, messages: Vec<Vec<u8>>) {
	for message in messages {
		host.send_dev_tools_message(Some(&message));
	}
}

/// The numbers in the JSON object `text`, each named `prefix` and its key.
fn numbers(prefix: &str, text: &str) -> Vec<(String, u64)> {
	let Ok(Json::Object(fields)) = Json::parse(text) else { return Vec::new() };
	fields
		.into_iter()
		.filter_map(|(k, v)| match v {
			Json::Int(n) => Some((format!("{prefix}{k}"), u64::try_from(n).ok()?)),
			Json::Float(f) if f >= 0.0 => Some((format!("{prefix}{k}"), f as u64)),
			_ => None,
		})
		.collect()
}

impl Flight {
	/// After the page's figures: the heap snapshot if one was asked for, else done.
	fn heap_snapshot_or_done(&mut self) {
		let Some((path, file, _)) = &mut self.heap else {
			self.step = Step::Done;
			return;
		};
		match std::fs::File::create(&*path) {
			Ok(f) => {
				*file = Some(f);
				self.send("HeapProfiler.enable", Json::Object(Vec::new()));
				let params = Json::Object(vec![("reportProgress".into(), Json::Bool(false))]);
				self.waiting = self.send("HeapProfiler.takeHeapSnapshot", params);
				self.step = Step::HeapSnapshot;
			}
			Err(e) => {
				self.errors.push(format!("the heap snapshot's file {}: {e}", path.display()));
				self.step = Step::Done;
			}
		}
	}

	/// Queue `method` with `params` to send ([`send_all`]); its id.
	fn send(&mut self, method: &str, params: Json) -> i32 {
		let id = self.next_id;
		self.next_id += 1;
		let message = Json::Object(vec![
			("id".into(), Json::Int(i64::from(id))),
			("method".into(), Json::Str(method.into())),
			("params".into(), params),
		]);
		self.outbox.push(message.to_text().into_bytes());
		id
	}

	fn finish(self) -> ChromiumDump {
		let (trace, processes, mut errors) = super::memory_dump_trace::read(&self.collected);
		errors.splice(0..0, self.errors);
		ChromiumDump {
			trace,
			processes,
			dumped: self.dumped,
			page: self.page,
			heap_snapshot: self.heap.filter(|(_, _, bytes)| *bytes > 0).map(|(path, _, bytes)| (path, bytes)),
			errors,
			took: self.sent.elapsed(),
		}
	}
}

// Passes DevTools' answers and events on to the dump on its way.
wrap_dev_tools_message_observer! {
	pub(super) struct DumpObserver {
		shared: std::sync::Arc<Shared>,
	}

	impl DevToolsMessageObserver {
		fn on_dev_tools_method_result(
			&self,
			browser: Option<&mut Browser>,
			message_id: ::std::os::raw::c_int,
			success: ::std::os::raw::c_int,
			result: Option<&[u8]>,
		) {
			let host = browser.and_then(|b| b.host());
			self.shared.memory_dumps.answered(host.as_ref(), message_id, success != 0, result.unwrap_or_default());
		}

		fn on_dev_tools_event(
			&self,
			browser: Option<&mut Browser>,
			method: Option<&CefString>,
			params: Option<&[u8]>,
		) {
			let Some(method) = method.map(|m| m.to_string()) else { return };
			let host = browser.and_then(|b| b.host());
			self.shared.memory_dumps.event(host.as_ref(), &method, params.unwrap_or_default());
		}
	}
}
