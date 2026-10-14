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
//! So at a discard point the renderer is told memory is short, which drops the decoded
//! copies (the pictures still shown are decoded again as they're drawn); V8 runs a
//! memory-reducing collection; and the renderer's allocator gives its free memory back
//! ([`trim`]). These arrive when the page runs next, in no set order: what a collection
//! frees after the trim goes at the next discard point.

use cef::*;

use super::ipc;

/// Have the page's renderer give back what it keeps for pictures and pages no longer
/// in use, and its allocator's free memory (`frame`: the page's, which carries the
/// message to its process).
pub(super) fn release(host: &BrowserHost, frame: Option<&Frame>) {
	let mut pressure = dictionary_value_create();
	if let Some(params) = pressure.as_mut() {
		params.set_string(Some(&"level".into()), Some(&"critical".into()));
	}
	host.execute_dev_tools_method(
		0,
		Some(&"Memory.simulatePressureNotification".into()),
		pressure.as_mut(),
	);
	host.execute_dev_tools_method(0, Some(&"HeapProfiler.collectGarbage".into()), None);
	if let Some(frame) = frame {
		ipc::send(frame, ProcessId::RENDERER, ipc::TRIM, &[]);
	}
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
