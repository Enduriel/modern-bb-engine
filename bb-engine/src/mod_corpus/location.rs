//! Where the reference corpus is: the folder `BB_MOD_CORPUS` names, else the first
//! folder named `mod-corpus` beside the checkout or beside one of the folders above it
//! (so every worktree of a checkout finds the same one). Machine-specific, like the game
//! install: never in the repository, whose files are public while the mods are their
//! authors'.

use std::path::{Path, PathBuf};

/// The variable naming the corpus folder.
pub const VAR: &str = "BB_MOD_CORPUS";
/// The corpus folder's name, looked for beside the checkout and the folders above it.
pub const FOLDER: &str = "mod-corpus";

/// The corpus folder, or why there is none (where it was looked for).
pub fn find() -> Result<PathBuf, String> {
	if let Some(dir) = std::env::var_os(VAR).filter(|v| !v.is_empty()).map(PathBuf::from) {
		return match dir.is_dir() {
			true => Ok(dir),
			false => Err(format!("{VAR} names {}, which is no folder", dir.display())),
		};
	}
	// The test's package folder (cargo and the test runner set it), else the current one.
	let start = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).or_else(|| std::env::current_dir().ok());
	let Some(start) = start else {
		return Err(format!("{VAR} is unset and there's no folder to look for {FOLDER} from"));
	};
	in_or_above(&start)
		.ok_or_else(|| format!("{VAR} is unset and no folder named {FOLDER} is in {} or a folder above it", start.display()))
}

/// The first `mod-corpus` folder in `start` or one of the folders above it.
pub fn in_or_above(start: &Path) -> Option<PathBuf> {
	start.ancestors().map(|dir| dir.join(FOLDER)).find(|dir| dir.is_dir())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_nearest_folder_above_is_found() {
		let root = std::env::temp_dir().join(format!("bb-corpus-location-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let checkout = root.join("project/code/worktrees/branch");
		std::fs::create_dir_all(&checkout).unwrap();
		assert!(in_or_above(&checkout).is_none_or(|p| !p.starts_with(&root)));
		std::fs::create_dir_all(root.join(FOLDER)).unwrap();
		assert_eq!(in_or_above(&checkout), Some(root.join(FOLDER)));
		std::fs::create_dir_all(root.join("project/code").join(FOLDER)).unwrap();
		assert_eq!(in_or_above(&checkout), Some(root.join("project/code").join(FOLDER)));
		// A file of that name is no corpus.
		std::fs::write(checkout.join(FOLDER), "").unwrap();
		assert_eq!(in_or_above(&checkout), Some(root.join("project/code").join(FOLDER)));
		std::fs::remove_dir_all(&root).unwrap();
	}
}
