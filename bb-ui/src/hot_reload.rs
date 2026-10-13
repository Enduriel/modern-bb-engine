//! Hot reload for UI development: notice that the archives the UI is served
//! from changed, e.g. a mod author rebuilt their `mod_*.zip`. The host then reopens them
//! ([`UiFiles::reopen`](crate::coui::UiFiles::reopen)) and restyles the running page
//! (`UiSession::reload_styles`), without a game restart.
//!
//! It polls modification times (std only), which is plenty for a person saving files.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// Watches a fixed set of files.
pub struct FileWatcher {
	/// Each file's modification time when last looked at (`None`: missing).
	seen: HashMap<PathBuf, Option<SystemTime>>,
}

impl FileWatcher {
	/// Watch `files`. How they are now is the baseline: only later changes are reported.
	pub fn new(files: impl IntoIterator<Item = PathBuf>) -> FileWatcher {
		let seen = files
			.into_iter()
			.map(|path| {
				let modified = modified(&path);
				(path, modified)
			})
			.collect();
		FileWatcher { seen }
	}

	/// The files that changed, appeared or disappeared since the last call, sorted.
	pub fn changed(&mut self) -> Vec<PathBuf> {
		let mut changed: Vec<PathBuf> = self
			.seen
			.iter_mut()
			.filter_map(|(path, seen)| {
				let now = modified(path);
				(now != *seen).then(|| {
					*seen = now;
					path.clone()
				})
			})
			.collect();
		changed.sort();
		changed
	}
}

fn modified(path: &PathBuf) -> Option<SystemTime> {
	std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
	use super::FileWatcher;
	use std::time::{Duration, SystemTime};

	#[test]
	fn reports_files_changed_after_the_baseline() {
		let dir = std::env::temp_dir().join(format!("bb-ui-watch-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let (a, b, c) = (
			dir.join("mod_a.zip"),
			dir.join("mod_b.zip"),
			dir.join("mod_c.zip"),
		);
		std::fs::write(&a, "a").unwrap();
		std::fs::write(&b, "b").unwrap();

		let mut watcher = FileWatcher::new([a.clone(), b.clone(), c.clone()]);
		assert!(watcher.changed().is_empty());

		// Modification times can be coarse: set one explicitly.
		let file = std::fs::File::options().write(true).open(&a).unwrap();
		file.set_modified(SystemTime::now() + Duration::from_secs(5))
			.unwrap();
		std::fs::write(&c, "c").unwrap();
		assert_eq!(watcher.changed(), [a, c]);
		assert!(watcher.changed().is_empty());

		std::fs::remove_dir_all(&dir).unwrap();
	}
}
