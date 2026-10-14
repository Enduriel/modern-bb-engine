//! The per-user cache folder on each system: `$XDG_CACHE_HOME` (else
//! `~/.cache`) on Linux and the like, `%LOCALAPPDATA%` on Windows, `~/Library/Caches`
//! on macOS; the engine's caches go in `bb-modern-engine/` there.

use std::path::PathBuf;

/// The engine's folder in the system's cache folder, or `None` without one.
pub fn engine_cache_dir() -> Option<PathBuf> {
	cache_home().map(|dir| dir.join("bb-modern-engine"))
}

#[cfg(windows)]
fn cache_home() -> Option<PathBuf> {
	std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn cache_home() -> Option<PathBuf> {
	std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"))
}

/// `$XDG_CACHE_HOME` if it's an absolute path (the spec ignores a relative one),
/// otherwise `~/.cache`.
#[cfg(not(any(windows, target_os = "macos")))]
fn cache_home() -> Option<PathBuf> {
	let xdg = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from);
	xdg.filter(|dir| dir.is_absolute())
		.or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
}
