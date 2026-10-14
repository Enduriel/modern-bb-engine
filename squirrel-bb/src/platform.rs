//! Where things are on this machine: the original game's install, the documents folder
//! it saves in, and the developer's copies of its saves. The first step of the
//! platform layer: Linux with Steam (the original under Proton) for now,
//! each path overridable from the environment.

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
