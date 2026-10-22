//! The game in Steam's libraries, found with `steamlocate`: Steam's folders (on Linux
//! every one it finds: the native client's, Flatpak's, Snap's; elsewhere the one the
//! system says: Windows' registry, macOS's `~/Library/Application Support/Steam`),
//! each one's libraries (`libraryfolders.vdf`), and the game's folder in each library
//! that has its manifest (`appmanifest_365360.acf`). Every library is tried, not only
//! the first with a manifest: a manifest left behind by a moved install doesn't hide
//! the real one.
//!
//! `BB_STEAM_DIR` names Steam's folder instead of looking for it (a folder without
//! Steam in it then finds nothing: how the tests hide this machine's install); the
//! caller reads it ([`super::Lookup::from_env`]).

use std::path::{Path, PathBuf};

use super::{Looked, Source, layout};

/// Where what Steam has is looked for.
const STEAM: Source = Source::Steam;

/// The game's Steam app id.
pub const APP_ID: u32 = 365_360;

/// The variable naming Steam's folder.
pub const STEAM_DIR_VAR: &str = "BB_STEAM_DIR";

/// The game's data folders in the libraries of Steam's folder `steam_dir`, or of every
/// Steam folder found without one, in the order found; what was looked at on the way
/// goes to `looked` (for the player's message when none is).
pub fn data_dirs(steam_dir: Option<&Path>, looked: &mut Vec<Looked>) -> Vec<PathBuf> {
	let steam_dirs = match steam_dir {
		Some(dir) => vec![dir.to_path_buf()],
		None => match steamlocate::locate_all() {
			Ok(dirs) => dirs.into_iter().map(|d| d.path().to_path_buf()).collect(),
			Err(e) => {
				looked.push(Looked::new(STEAM, "Steam", format!("not found ({e})")));
				return Vec::new();
			}
		},
	};
	if steam_dirs.is_empty() {
		looked.push(Looked::new(STEAM, "Steam", "not found"));
	}
	steam_dirs.iter().flat_map(|dir| in_steam_dir(dir, looked)).collect()
}

/// The game's data folders in the libraries of the Steam folder `dir`.
fn in_steam_dir(dir: &Path, looked: &mut Vec<Looked>) -> Vec<PathBuf> {
	let place = format!("Steam in {}", dir.display());
	if !dir.join("steamapps").is_dir() {
		looked.push(Looked::new(STEAM, place, "not installed there"));
		return Vec::new();
	}
	let steam = match steamlocate::SteamDir::from_dir(dir) {
		Ok(steam) => steam,
		Err(e) => {
			looked.push(Looked::new(STEAM, place, format!("no Steam there ({e})")));
			return Vec::new();
		}
	};
	let libraries = match steam.libraries() {
		Ok(libraries) => libraries,
		Err(e) => {
			looked.push(Looked::new(STEAM, place, format!("its library list can't be read ({e})")));
			return Vec::new();
		}
	};
	let mut found = Vec::new();
	let mut listed = false;
	for library in libraries {
		let library = match library {
			Ok(library) => library,
			Err(e) => {
				looked.push(unreadable_library(&place, e));
				continue;
			}
		};
		let Some(app) = library.app(APP_ID) else { continue };
		listed = true;
		let place = format!("Steam library {}", library.path().display());
		match app {
			Ok(app) => {
				let game = library.resolve_app_dir(&app);
				match layout::data_dir_in(&game) {
					Some(data) => found.push(data),
					None => looked.push(Looked::new(
						STEAM,
						place,
						format!("the game should be in {}, but no data/{} is there", game.display(), layout::MARKER),
					)),
				}
			}
			Err(e) => looked.push(Looked::new(STEAM, place, format!("the game's manifest can't be read ({e})"))),
		}
	}
	if !listed {
		looked.push(Looked::new(STEAM, place, "Battle Brothers isn't installed in any of its libraries"));
	}
	found
}

/// Why a library Steam lists was skipped. A library that isn't there at all (a drive
/// that's gone, unplugged or now something else: Steam keeps listing it) is said as
/// such, not as an error reading Steam.
fn unreadable_library(steam_place: &str, e: steamlocate::Error) -> Looked {
	match e {
		steamlocate::Error::Io { inner, path } if inner.kind() == std::io::ErrorKind::NotFound => {
			let library = if path.ends_with("steamapps") { path.parent().unwrap_or(&path) } else { &path };
			Looked::new(
				STEAM,
				format!("Steam library {}", library.display()),
				"Steam lists it, but it isn't there (a drive or folder that's gone)",
			)
		}
		e => Looked::new(STEAM, steam_place, format!("a library can't be read ({e})")),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::install::test_tree::Tree;

	#[test]
	fn the_game_is_found_in_a_steam_folders_library() {
		let tree = Tree::new("steam_found");
		let steam = tree.steam("Steam", "Battle Brothers", true);
		let mut looked = Vec::new();
		let found = in_steam_dir(&steam, &mut looked);
		assert_eq!(found, vec![steam.join("steamapps/common/Battle Brothers/data")], "{looked:?}");
		assert!(looked.is_empty(), "{looked:?}");
	}

	#[test]
	fn a_manifest_without_the_files_says_so() {
		let tree = Tree::new("steam_uninstalled");
		let steam = tree.steam("Steam", "Battle Brothers", false);
		let mut looked = Vec::new();
		assert!(in_steam_dir(&steam, &mut looked).is_empty());
		assert_eq!(looked.len(), 1, "{looked:?}");
		assert!(looked[0].outcome.contains("no data/data_001.dat"), "{looked:?}");
	}

	#[test]
	fn a_listed_library_that_is_gone_is_skipped_as_such() {
		let tree = Tree::new("steam_library_gone");
		let steam = tree.steam("Steam", "Battle Brothers", true);
		let gone = tree.root().join("gone");
		let vdf = steam.join("steamapps/libraryfolders.vdf");
		let folders = std::fs::read_to_string(&vdf).unwrap();
		let entry = format!("\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n", gone.display().to_string().replace('\\', "\\\\"));
		std::fs::write(&vdf, folders.trim_end().strip_suffix('}').unwrap().to_string() + &entry).unwrap();
		let mut looked = Vec::new();
		let found = in_steam_dir(&steam, &mut looked);
		assert_eq!(found, vec![steam.join("steamapps/common/Battle Brothers/data")], "{looked:?}");
		assert_eq!(looked.len(), 1, "{looked:?}");
		assert_eq!(looked[0].place, format!("Steam library {}", gone.display()), "{looked:?}");
		assert!(looked[0].outcome.contains("isn't there"), "{looked:?}");
	}

	#[test]
	fn a_folder_without_steam_finds_nothing() {
		let tree = Tree::new("steam_none");
		let mut looked = Vec::new();
		assert!(in_steam_dir(tree.root(), &mut looked).is_empty());
		assert_eq!(looked.len(), 1, "{looked:?}");
		assert_eq!(looked[0].outcome, "not installed there", "{looked:?}");
	}
}
