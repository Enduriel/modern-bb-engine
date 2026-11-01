//! Directories mounted at the root of the game's file system, beside the archives.
//!
//! The original's PhysFS search path holds the install folder and its `data` folder as
//! well as the archives (verified against the original): `IO.enumerateFiles("data/")`
//! lists the real data folder (`data/data_001`, `data/mod_msu-…`, loose `data/gfx/fonts/…`),
//! which Reforged and Modular Vanilla read to detect conflicting mods, and the data
//! folder's loose files appear at the root (`gfx/fonts/…`), where MSU looks for a
//! user's `mod_config/<mod>/` files. Like PhysFS, a path with a `.` or `..` component
//! names nothing.
//!
//! The data folder is searched before the archives (probed 2026-11-01): a loose
//! `data/scripts/x.nut` replaces every archive's `scripts/x.nut`. The original also
//! searches the install folder, after the archives; we don't (user, 2026-11-01): only the
//! data folder's own files are read, and they are also listed under `data/`, as the
//! install folder lists them in the original.

use std::path::{Path, PathBuf};

/// The mounted directories, searched before the archives, and directories only listed
/// under a prefix.
#[derive(Default)]
pub struct Mounts {
	dirs: Vec<PathBuf>,
	/// (prefix ending in `/`, directory): files `IO.enumerateFiles` lists under the
	/// prefix, never read through it.
	listed: Vec<(String, PathBuf)>,
}

impl Mounts {
	/// `dirs` mounted at the root, searched in order before the archives.
	pub fn new(dirs: Vec<PathBuf>) -> Self {
		Mounts { dirs, listed: Vec::new() }
	}

	/// Also list `dir`'s files under `prefix` (ending in `/`; e.g. the data folder as
	/// `data/`), without making them readable there.
	pub fn listed_at(mut self, prefix: &str, dir: PathBuf) -> Self {
		self.listed.push((prefix.to_string(), dir));
		self
	}

	/// The directories mounted at the root, in search order.
	pub fn dirs(&self) -> &[PathBuf] {
		&self.dirs
	}

	/// Every file under the directory `dir` (a file-system path, empty or ending in `/`)
	/// in each mounted directory, recursively, as file-system paths with their
	/// extensions, the listed-only directories' included. Unreadable entries are skipped.
	pub fn files_under(&self, dir: &str) -> Vec<String> {
		let mut out = Vec::new();
		if !valid(dir.trim_end_matches('/')) && !dir.is_empty() {
			return out;
		}
		for root in &self.dirs {
			walk(&root.join(dir), dir, &mut out);
		}
		for (prefix, root) in &self.listed {
			if let Some(below) = dir.strip_prefix(prefix.as_str()) {
				walk(&root.join(below), dir, &mut out);
			} else if prefix.starts_with(dir) {
				walk(root, prefix, &mut out);
			}
		}
		out
	}

	/// The real file at `path` (with its extension) in the first mounted directory that
	/// has it.
	pub fn find(&self, path: &str) -> Option<PathBuf> {
		if !valid(path) {
			return None;
		}
		self.dirs.iter().map(|root| root.join(path)).find(|file| file.is_file())
	}

	/// The bytes of the file at `path` from the first mounted directory that has it.
	pub fn read(&self, path: &str) -> Option<Vec<u8>> {
		std::fs::read(self.find(path)?).ok()
	}
}

/// Every file under the real directory `root`, recursively, as paths relative to it.
/// Unreadable entries are skipped.
pub fn files_of(root: &Path) -> Vec<String> {
	let mut out = Vec::new();
	walk(root, "", &mut out);
	out
}

/// When the real file `file` was last modified, as Unix seconds (as the original reads a
/// loose file's time: whole seconds).
pub fn modified(file: &Path) -> Option<i64> {
	let time = std::fs::metadata(file).ok()?.modified().ok()?;
	Some(match time.duration_since(std::time::UNIX_EPOCH) {
		Ok(after) => after.as_secs() as i64,
		Err(before) => -(before.duration().as_secs_f64().ceil() as i64),
	})
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

	#[test]
	fn a_listed_directory_is_listed_under_its_prefix_but_not_read_there() {
		let root = std::env::temp_dir().join(format!("bb-mounts-listed-{}", std::process::id()));
		std::fs::create_dir_all(root.join("gfx")).unwrap();
		std::fs::write(root.join("mod_a.zip"), b"z").unwrap();
		std::fs::write(root.join("gfx/f.ttf"), b"f").unwrap();
		let mounts = Mounts::new(vec![root.clone()]).listed_at("data/", root.clone());
		let sorted = |mut v: Vec<String>| {
			v.sort();
			v
		};
		assert_eq!(sorted(mounts.files_under("data/")), ["data/gfx/f.ttf", "data/mod_a.zip"]);
		assert_eq!(mounts.files_under("data/gfx/"), ["data/gfx/f.ttf"]);
		assert_eq!(sorted(mounts.files_under("")), ["data/gfx/f.ttf", "data/mod_a.zip", "gfx/f.ttf", "mod_a.zip"]);
		assert_eq!(mounts.read("data/mod_a.zip"), None);
		assert_eq!(mounts.read("gfx/f.ttf").as_deref(), Some(&b"f"[..]));
		std::fs::remove_dir_all(root).unwrap();
	}
}
