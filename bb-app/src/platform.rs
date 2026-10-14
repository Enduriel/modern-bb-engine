//! What differs between the systems the window runs on, one small function each.

use winit::event_loop::ActiveEventLoop;

/// Whether a window can take the monitor in a mode of its own (exclusive fullscreen).
/// Wayland has no such thing (winit ignores it there).
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub fn exclusive_fullscreen(event_loop: &ActiveEventLoop) -> bool {
	use winit::platform::wayland::ActiveEventLoopExtWayland;
	!event_loop.is_wayland()
}

#[cfg(not(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))))]
pub fn exclusive_fullscreen(_event_loop: &ActiveEventLoop) -> bool {
	true
}

/// The smallest buffer the allocator always takes from the system and gives back when
/// freed: the size of the smallest atlas (2048×1024 RGBA).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const SYSTEM_BUFFER: i32 = 8 << 20;

/// How many arenas (free lists) the C allocator keeps for the process's threads.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const ARENAS: i32 = 2;

/// Tune the C allocator for the game's work, before any thread starts (glibc):
/// - Atlases decode on worker threads into buffers of 8 to 64 MiB, which the game's
///   thread frees after uploading them. glibc raises its threshold for taking a buffer
///   straight from the system after each such free (up to 32 MiB), so later buffers
///   come from the workers' arenas instead, and stay held there once freed: on a fresh
///   world (measured 2026-10-14) 393 MB resident, 304 MB with the
///   threshold fixed at 8 MiB (`M_MMAP_THRESHOLD`; the frame cost unchanged). Buffers
///   below it (a frame's vertices, the scripts') stay with the allocator.
/// - glibc gives each thread an arena of its own, up to 8 per core, and memory freed in
///   one stays there: the decode workers', the sound loaders', and those of the
///   browser's threads running in the game's process. Two arenas for all of them
///   (`M_ARENA_MAX`), and for the browser's own processes too (`MALLOC_ARENA_MAX`, read
///   as each starts): with the UI, a fresh world 650 -> 615 MB (PSS, every process),
///   and after three battles 1425 -> 837, the frame cost unchanged; four arenas
///   saved about half as much.
/// Other systems' allocators give buffers this large back by themselves.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn tune_allocator() {
	/// glibc's `mallopt` parameters (`malloc.h`).
	const M_MMAP_THRESHOLD: i32 = -3;
	const M_ARENA_MAX: i32 = -8;
	unsafe extern "C" {
		fn mallopt(param: i32, value: i32) -> i32;
	}
	// SAFETY: `mallopt` only sets the allocator's parameters, and setting the variable
	// is sound with no other thread running: both happen before any starts.
	unsafe {
		if mallopt(M_MMAP_THRESHOLD, SYSTEM_BUFFER) != 1 || mallopt(M_ARENA_MAX, ARENAS) != 1 {
			eprintln!("allocator: could not set its parameters");
		}
		std::env::set_var("MALLOC_ARENA_MAX", ARENAS.to_string());
	}
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn tune_allocator() {}
