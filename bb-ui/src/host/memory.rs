//! The page gives memory back where the game discards a scene: a
//! battle's end, the campaign left, a save loaded, as the renderer's textures and the
//! sound bank do.
//!
//! What a session's page holds beyond its live objects (measured over battles, 2026-10-14):
//! - the compositor's decoded copies of the pictures it drew, a full-screen picture
//!   7.9MB each (the loading screens' backgrounds, a new one each load): Chromium keeps
//!   them until memory runs short;
//! - V8's pages: its collections run often, but only a memory-reducing one gives freed
//!   pages back to the system, and V8 starts one when the page goes quiet, which a
//!   game's page never does. Its footprint stays at the busiest battle's;
//! - the C allocator's free memory: glibc gives back only what is free at the top of an
//!   arena, so what the renderer frees in the middle stays held, ~12MB more a battle.
//!
//! So at a discard point the renderer is told memory is getting short ("moderate"
//! pressure); V8 runs a memory-reducing collection; and the renderer's allocator gives
//! its free memory back ([`trim`]). These arrive when the page runs next, in no set
//! order: what a collection frees after the trim goes at the next discard point. They
//! run on the renderer's main thread, so the page can't park meanwhile: a game loop
//! goes on drawing without its scripts until it has (`UiSession::park_for_frame`).
//!
//! The parts compared (2026-10-23, a modded campaign: battle, quit to the menu, load,
//! four times; the renderer's PSS 7 s after reaching the menu / the loaded world, two
//! runs each, `BB_PAGE_RELEASE`): none 201-219 / 200-212 MB; critical pressure, the
//! collection and the trim (the default until then) 187-189 / 174-183 MB, 30-60 ms a
//! release; moderate pressure, the collection and the trim 174-176 / 171-173 MB, 35-60
//! ms; the trim alone 191-192 / 182-186 MB, 1-8 ms. Critical pressure also throws away
//! the shown pictures' decodes, which the menu then decodes again. On a machine short
//! of memory (the renderer swapping) a release took 1.1-1.6 s, the trim alone up to
//! 1.2 s.

use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Instant;

use cef::*;

use super::ipc;
use super::memory_flight::Flights;

/// What a release does (`BB_PAGE_RELEASE`): which memory-pressure signal the renderer
/// gets, if any; whether V8 runs a memory-reducing collection; whether the renderer's
/// allocator gives its free memory back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Parts {
	pub pressure: Option<&'static str>,
	pub collect: bool,
	pub trim: bool,
}

impl Parts {
	/// What a release does by default.
	pub const DEFAULT: Parts = Parts { pressure: Some("moderate"), collect: true, trim: true };

	/// None: no release at all.
	pub const NONE: Parts = Parts { pressure: None, collect: false, trim: false };

	/// `BB_PAGE_RELEASE`: unset, empty or `1` is [`DEFAULT`](Self::DEFAULT), `0` none;
	/// otherwise the parts it lists, comma-separated (`critical` or `moderate`, `gc`,
	/// `trim`), to compare them. Unknown names are ignored.
	pub fn from_env() -> Parts {
		Parts::parse(std::env::var("BB_PAGE_RELEASE").ok().as_deref())
	}

	fn parse(value: Option<&str>) -> Parts {
		match value.map(str::trim) {
			None | Some("") | Some("1") => Parts::DEFAULT,
			Some("0") => Parts::NONE,
			Some(list) => {
				let mut parts = Parts::NONE;
				for name in list.split(',').map(str::trim) {
					match name {
						"critical" => parts.pressure = Some("critical"),
						"moderate" => parts.pressure = Some("moderate"),
						"gc" => parts.collect = true,
						"trim" => parts.trim = true,
						_ => {}
					}
				}
				parts
			}
		}
	}

	pub fn any(self) -> bool {
		self != Parts::NONE
	}
}

/// The DevTools message ids the releases' calls take: their own range, so their answers
/// can be told apart. Called off CEF's UI thread, `execute_dev_tools_method` posts the
/// call there and returns 0, not the id it would assign.
static NEXT_MESSAGE: AtomicI32 = AtomicI32::new(1 << 24);

/// Have the page's renderer give back what it keeps for pictures and pages no longer
/// in use, and its allocator's free memory, as `parts` says (`frame`: the page's, which
/// carries the message to its process). The answers reach `flights` (DevTools' only if
/// `observed`: an observer passes them on), which times it.
pub(super) fn release(host: &BrowserHost, frame: Option<&Frame>, parts: Parts, flights: &Flights, observed: bool) {
	let sent = Instant::now();
	let mut methods = Vec::new();
	let mut call = |method: &str, params: Option<&mut DictionaryValue>, part: &'static str| {
		let id = NEXT_MESSAGE.fetch_add(1, Ordering::Relaxed);
		host.execute_dev_tools_method(id, Some(&method.into()), params);
		if observed {
			methods.push((id, part));
		}
	};
	if let Some(level) = parts.pressure {
		let mut pressure = dictionary_value_create();
		if let Some(params) = pressure.as_mut() {
			params.set_string(Some(&"level".into()), Some(&level.into()));
		}
		call("Memory.simulatePressureNotification", pressure.as_mut(), "pressure");
	}
	if parts.collect {
		call("HeapProfiler.collectGarbage", None, "collect");
	}
	let trim = parts.trim && frame.is_some_and(|frame| ipc::send(frame, ProcessId::RENDERER, ipc::TRIM, &[]));
	flights.start(sent, methods, trim);
}

/// In the renderer: give the C allocator's free memory back to the system, from every
/// arena (glibc's `malloc_trim`; other systems' allocators do it themselves).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(super) fn trim() {
	unsafe extern "C" {
		fn malloc_trim(pad: usize) -> std::ffi::c_int;
	}
	// SAFETY: `malloc_trim` only returns free pages to the system; any thread may call it.
	unsafe {
		malloc_trim(0);
	}
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub(super) fn trim() {}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_release_is_chosen_by_its_variable() {
		assert_eq!(Parts::parse(None), Parts::DEFAULT);
		assert_eq!(Parts::parse(Some("1")), Parts::DEFAULT);
		assert_eq!(Parts::parse(Some("0")), Parts::NONE);
		assert!(!Parts::parse(Some("0")).any());
		let moderate = Parts::parse(Some("moderate, trim"));
		assert_eq!(moderate, Parts { pressure: Some("moderate"), collect: false, trim: true });
		assert_eq!(Parts::parse(Some("gc")), Parts { pressure: None, collect: true, trim: false });
	}
}
