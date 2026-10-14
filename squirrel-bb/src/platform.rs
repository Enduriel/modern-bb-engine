//! Where things are on this machine: the original game's install, the documents folder
//! it saves in, the developer's copies of its saves, and this engine's own saves. The
//! first step of the platform layer: the original's paths are Linux with
//! Steam (the original under Proton) for now, ours are each system's; each path can be
//! overridden from the environment.

use std::path::PathBuf;

/// The original's Steam app id.
const APP_ID: &str = "365360";

/// The user's home directory (`$HOME`).
fn home() -> Option<PathBuf> {
	std::env::var_os("HOME").map(PathBuf::from)
}

/// Steam's directory: `$BB_STEAM_DIR`, else `~/.local/share/Steam`.
pub fn steam_dir() -> Option<PathBuf> {
	std::env::var_os("BB_STEAM_DIR").map(PathBuf::from).or_else(|| Some(home()?.join(".local/share/Steam")))
}

/// The original game's install directory.
pub fn game_dir() -> Option<PathBuf> {
	Some(steam_dir()?.join("steamapps/common/Battle Brothers"))
}

/// The folder the original keeps its log, settings and saves in (its Windows
/// `Documents/Battle Brothers`, inside Proton's prefix).
pub fn original_documents_dir() -> Option<PathBuf> {
	Some(steam_dir()?.join("steamapps/compatdata").join(APP_ID).join("pfx/drive_c/users/steamuser/Documents/Battle Brothers"))
}

/// The original's savegames folder (the player's saves; probes' saves come and go there).
pub fn original_savegames_dir() -> Option<PathBuf> {
	Some(original_documents_dir()?.join("savegames"))
}

/// Saves the original wrote, kept for the tests of its format:
/// `$BB_ORIGINAL_SAVES`. Not in the
/// repository (each is ~6.7 MB); tests skip without them.
pub fn original_save_fixtures() -> Option<PathBuf> {
	std::env::var_os("BB_ORIGINAL_SAVES").map(PathBuf::from)
}

/// Our folder in the system's data folder.
const FOLDER: &str = "battle-brothers-modern";

/// Where this engine keeps its saves: `$BB_SAVES_DIR`, else `savegames` in our folder
/// in the per-user data folder (`%APPDATA%` on Windows, `~/Library/Application
/// Support` on macOS, the XDG data folder elsewhere). Never the original's savegames
/// folder, whose game would list files it can't read.
pub fn saves_dir() -> Option<PathBuf> {
	std::env::var_os("BB_SAVES_DIR")
		.map(PathBuf::from)
		.or_else(|| Some(data_home()?.join(FOLDER).join("savegames")))
}

#[cfg(windows)]
fn data_home() -> Option<PathBuf> {
	std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn data_home() -> Option<PathBuf> {
	Some(home()?.join("Library/Application Support"))
}

/// `$XDG_DATA_HOME` if it's an absolute path (the spec ignores a relative one),
/// otherwise `~/.local/share`.
#[cfg(not(any(windows, target_os = "macos")))]
fn data_home() -> Option<PathBuf> {
	let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
	xdg.filter(|dir| dir.is_absolute()).or_else(|| Some(home()?.join(".local/share")))
}
