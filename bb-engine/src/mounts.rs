//! Directories mounted at the root of the game's file system, beside the archives.
//!
//! The original's PhysFS search path holds the install folder and its `data` folder as
//! well as the archives (verified against the original): `IO.enumerateFiles("data/")`
//! lists the real data folder (`data/data_001`, `data/mod_msu-…`, loose `data/gfx/fonts/…`),
//! which Reforged and Modular Vanilla read to detect conflicting mods, and the data
//! folder's loose files appear at the root (`gfx/fonts/…`), where MSU looks for a
//! user's `mod_config/<mod>/` files. Like PhysFS, a path with a `.` or `..` component
//! names nothing.

use std::path::{Path, PathBuf};

/// The mounted directories, searched in order.
#[derive(Default)]
pub struct Mounts {
	dirs: Vec<PathBuf>,
}

impl Mounts {
	pub fn new(dirs: Vec<PathBuf>) -> Self {
		Mounts { dirs }
	}

	/// Every file under the directory `dir` (a file-system path, empty or ending in `/`)
	/// in each mounted directory, recursively, as file-system paths with their
	/// extensions. Unreadable entries are skipped.
	pub fn files_under(&self, dir: &str) -> Vec<String> {
		let mut out = Vec::new();
		if !valid(dir.trim_end_matches('/')) && !dir.is_empty() {
			return out;
		}
		for root in &self.dirs {
			walk(&root.join(dir), dir, &mut out);
		}
		out
	}

	/// The bytes of the file at `path` (with its extension), from the first mounted
	/// directory that has it.
	pub fn read(&self, path: &str) -> Option<Vec<u8>> {
		if !valid(path) {
			return None;
		}
		self.dirs.iter().find_map(|root| std::fs::read(root.join(path)).ok())
	}
}

/// Whether `path` is a relative path PhysFS accepts: no empty, `.` or `..` components
/// and no backslashes or drive letters.
fn valid(path: &str) -> bool {
	!path.is_empty()
		&& !path.contains(['\\', ':'])
		&& path.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// Collect the files under the real directory `real`, named `prefix` + their path
/// below it.
fn walk(real: &Path, prefix: &str, out: &mut Vec<String>) {
	let Ok(entries) = std::fs::read_dir(real) else { return };
	for entry in entries.flatten() {
		let name = entry.file_name().to_string_lossy().into_owned();
		let Ok(kind) = entry.file_type() else { continue };
		let path = format!("{prefix}{name}");
		if kind.is_dir() {
			walk(&entry.path(), &format!("{path}/"), out);
		} else {
			out.push(path);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn lists_and_reads_mounted_files() {
		let root = std::env::temp_dir().join(format!("bb-mounts-{}", std::process::id()));
		std::fs::create_dir_all(root.join("mod_config/msu")).unwrap();
		std::fs::write(root.join("mod_config/msu/settings.nut"), b"x").unwrap();
		std::fs::write(root.join("top.txt"), b"y").unwrap();
		let mounts = Mounts::new(vec![root.clone()]);
		assert_eq!(mounts.files_under("mod_config/"), vec!["mod_config/msu/settings.nut".to_string()]);
		let mut all = mounts.files_under("");
		all.sort();
		assert_eq!(all, vec!["mod_config/msu/settings.nut".to_string(), "top.txt".to_string()]);
		assert_eq!(mounts.read("top.txt").as_deref(), Some(&b"y"[..]));
		assert!(mounts.files_under("missing/").is_empty());
		// PhysFS rejects relative components.
		assert!(mounts.files_under("./mod_config/").is_empty());
		assert!(mounts.read("mod_config/../top.txt").is_none());
		std::fs::remove_dir_all(root).unwrap();
	}
}
