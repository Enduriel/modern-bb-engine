//! The original game's savegames folder on this system: `Battle Brothers\savegames` in
//! the player's Documents (the folder its `config.cfg`, `log.html` and saves are in).
//!
//! - Windows: the Documents known folder (`FOLDERID_Documents`, so a Documents moved
//!   elsewhere, OneDrive's included, is followed), whatever the install's store.
//! - Linux and the other Unixes: the original runs only under Proton there, so it's the
//!   Documents of the Proton prefix Steam made for the game, in the library of the Steam
//!   install bb found (`<steamapps>/compatdata/365360/pfx/drive_c/users/steamuser`).
//!   Only if that prefix is there: bb never makes one (Steam does, the first time the
//!   original runs). The folders under it (`Documents/Battle Brothers/savegames`) are
//!   made with the first save, as the original would make them.
//! - macOS: the original doesn't run there; never.

use std::path::{Path, PathBuf};

/// The savegames folder in the Documents folder `documents`.
fn in_documents(documents: PathBuf) -> PathBuf {
	documents.join("Battle Brothers").join("savegames")
}

/// The original's savegames folder for the install whose data folder is `data_dir`,
/// or why there's none for it.
#[cfg(windows)]
pub fn savegames_dir(_data_dir: &Path) -> Result<PathBuf, String> {
	Ok(in_documents(super::documents::documents_dir()?))
}

#[cfg(target_os = "macos")]
pub fn savegames_dir(_data_dir: &Path) -> Result<PathBuf, String> {
	Err("the original game doesn't run on macOS".to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn savegames_dir(data_dir: &Path) -> Result<PathBuf, String> {
	in_proton_prefix(data_dir)
}

/// The savegames folder in the Proton prefix of the Steam install whose data folder is
/// `data_dir`, or why there's none (not a Steam install, or no prefix: the original has
/// never run there).
pub fn in_proton_prefix(data_dir: &Path) -> Result<PathBuf, String> {
	let install = bb_steam::install::steam_install(data_dir)?;
	let user = proton_user_dir(&install.steamapps);
	if !user.is_dir() {
		return Err(format!("no Proton prefix of the original's at {} (it has never run there)", user.display()));
	}
	Ok(in_documents(user.join("Documents")))
}

/// The prefix's user folder for the game in the Steam library `steamapps`.
pub fn proton_user_dir(steamapps: &Path) -> PathBuf {
	steamapps
		.join("compatdata")
		.join(bb_steam::APP_ID.to_string())
		.join("pfx/drive_c/users/steamuser")
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::fs;

	fn scratch(name: &str) -> PathBuf {
		let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/tmp/bb-app-saves").join(name);
		let _ = fs::remove_dir_all(&dir);
		fs::create_dir_all(&dir).unwrap();
		dir
	}

	/// A fake Steam library with the game's data folder in it, and a GOG-style one.
	fn library(name: &str) -> (PathBuf, PathBuf, PathBuf) {
		let root = scratch(name);
		let steamapps = root.join("lib/steamapps");
		let data = steamapps.join("common/Battle Brothers/data");
		let gog = root.join("GOG Games/Battle Brothers/data");
		fs::create_dir_all(&data).unwrap();
		fs::create_dir_all(&gog).unwrap();
		fs::write(steamapps.join("appmanifest_365360.acf"), "\"AppState\"\n{\n\t\"installdir\"\t\t\"Battle Brothers\"\n}\n").unwrap();
		(fs::canonicalize(&steamapps).unwrap(), data, gog)
	}

	#[test]
	fn the_prefix_of_the_installs_own_library() {
		let (steamapps, data, _) = library("prefix");
		let user = proton_user_dir(&steamapps);
		fs::create_dir_all(&user).unwrap();
		assert_eq!(in_proton_prefix(&data), Ok(user.join("Documents/Battle Brothers/savegames")));
		// Nothing is made for it.
		assert!(!user.join("Documents").exists());
	}

	#[test]
	fn none_without_the_prefix_and_none_off_steam() {
		let (_, data, gog) = library("none");
		let why = in_proton_prefix(&data).unwrap_err();
		assert!(why.contains("no Proton prefix"), "{why}");
		assert!(in_proton_prefix(&gog).unwrap_err().contains("not in a Steam library"));
	}
}
