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
