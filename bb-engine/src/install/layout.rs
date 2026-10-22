//! Whether a folder holds the game: the check every candidate passes, wherever it came
//! from. The game's files are its `data_*.dat` archives, and the first of them,
//! `data_001.dat`, is always there; the folder bb runs is the one holding them (`data`).
//!
//! A folder can name the install in more ways than one, all taken:
//! - the game's folder, with `data/data_001.dat` in it (Steam's and GOG's Windows
//!   layout, and what a player picks);
//! - the `data` folder itself (what `--data` names);
//! - the game's `win32` folder, where its `BattleBrothers.exe` is, beside `data`;
//! - a macOS app bundle (`*.app`), or a folder with one in it, holding `data` in
//!   `Contents/Resources` or `Contents`. Not seen on a real Mac install yet: the layouts
//!   a bundle's resources take, so a Mac copy is found if it is one.

use std::path::{Path, PathBuf};

/// The file whose presence makes a folder the game's data folder.
pub const MARKER: &str = "data_001.dat";

/// The game's data folder, if `folder` names the install in one of the ways above.
pub fn data_dir_in(folder: &Path) -> Option<PathBuf> {
	let candidates = [folder.join("data"), folder.to_path_buf()]
		.into_iter()
		.chain(win32_sibling(folder))
		.chain(bundle_data_dirs(folder))
		.chain(bundles_in(folder).into_iter().flat_map(|bundle| bundle_data_dirs(&bundle)));
	candidates.into_iter().find(|dir| is_data_dir(dir))
}

/// Whether `dir` is a data folder: `data_001.dat` is a file in it.
pub fn is_data_dir(dir: &Path) -> bool {
	dir.join(MARKER).is_file()
}

/// `data` beside a `win32` folder.
fn win32_sibling(folder: &Path) -> Option<PathBuf> {
	let named = folder.file_name()?.to_string_lossy().eq_ignore_ascii_case("win32");
	named.then(|| folder.parent().map(|game| game.join("data")))?
}

/// Where an app bundle's data folder may be.
fn bundle_data_dirs(bundle: &Path) -> [PathBuf; 2] {
	let contents = bundle.join("Contents");
	[contents.join("Resources").join("data"), contents.join("data")]
}

/// The app bundles directly in `folder`, by name (a deterministic order).
fn bundles_in(folder: &Path) -> Vec<PathBuf> {
	let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
	let mut bundles: Vec<PathBuf> = entries
		.flatten()
		.map(|e| e.path())
		.filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) && p.is_dir())
		.collect();
	bundles.sort();
	bundles
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::install::test_tree::Tree;

	#[test]
	fn the_game_folder_its_data_folder_and_win32_all_name_it() {
		let tree = Tree::new("layout_windows");
		let game = tree.game("Battle Brothers");
		let data = game.join("data");
		assert_eq!(data_dir_in(&game), Some(data.clone()));
		assert_eq!(data_dir_in(&data), Some(data.clone()));
		std::fs::create_dir_all(game.join("win32")).unwrap();
		assert_eq!(data_dir_in(&game.join("win32")), Some(data));
	}

	#[test]
	fn a_folder_without_data_001_is_not_the_game() {
		let tree = Tree::new("layout_missing");
		let game = tree.root().join("Battle Brothers");
		std::fs::create_dir_all(game.join("data")).unwrap();
		std::fs::write(game.join("data").join("data_002.dat"), b"").unwrap();
		assert_eq!(data_dir_in(&game), None);
		assert_eq!(data_dir_in(&tree.root().join("nowhere")), None);
		// A folder named like the marker is not the marker.
		std::fs::create_dir_all(game.join("data").join(MARKER)).unwrap();
		assert_eq!(data_dir_in(&game), None);
	}

	#[test]
	fn a_mac_app_bundle_is_looked_into() {
		let tree = Tree::new("layout_mac");
		let game = tree.root().join("Battle Brothers");
		let data = game.join("Battle Brothers.app/Contents/Resources/data");
		std::fs::create_dir_all(&data).unwrap();
		std::fs::write(data.join(MARKER), b"").unwrap();
		assert_eq!(data_dir_in(&game), Some(data.clone()), "the game's folder, with the bundle in it");
		assert_eq!(data_dir_in(&game.join("Battle Brothers.app")), Some(data), "the bundle itself");
	}
}
