//! Where the settings file lives on each system: the per-user configuration folder
//! (`%APPDATA%` on Windows, `~/Library/Application Support` on macOS, the XDG config
//! folder elsewhere), in a folder of our own. The original's `config.cfg` is never
//! touched.

use std::path::PathBuf;

/// Our folder in the system's configuration folder.
const FOLDER: &str = "battle-brothers-modern";

/// The settings folder, or `None` without a home to put it in.
pub fn settings_dir() -> Option<PathBuf> {
	config_home().map(|dir| dir.join(FOLDER))
}

#[cfg(windows)]
fn config_home() -> Option<PathBuf> {
	std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn config_home() -> Option<PathBuf> {
	std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
}

/// `$XDG_CONFIG_HOME` if it's an absolute path (the spec ignores a relative one),
/// otherwise `~/.config`.
#[cfg(not(any(windows, target_os = "macos")))]
fn config_home() -> Option<PathBuf> {
	let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
	xdg.filter(|dir| dir.is_absolute())
		.or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}
