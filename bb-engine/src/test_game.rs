//! A game folder in the temp dir for tests: archives with chosen entry times, and loose
//! files in the data and install folders.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::datsource::DatScriptSource;
use crate::mounts::Mounts;

pub(crate) const OLD: (u16, u8) = (2000, 1);
pub(crate) const MID: (u16, u8) = (2020, 1);
pub(crate) const NEW: (u16, u8) = (2030, 1);

/// A game folder in the temp dir: `data/` (the data folder) inside an install folder.
pub(crate) struct Game {
	root: PathBuf,
	archives: Vec<PathBuf>,
}

impl Game {
	pub(crate) fn new(name: &str) -> Game {
		let root = std::env::temp_dir().join(format!("bb-test-game-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(root.join("data")).unwrap();
		Game { root, archives: Vec::new() }
	}

	/// Add an archive in `data/` (load order is the order added), its entries modified in
	/// the given (year, month).
	pub(crate) fn zip(&mut self, name: &str, entries: &[(&str, &[u8], (u16, u8))]) -> &mut Game {
		let path = self.root.join("data").join(name);
		let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
		for (entry, bytes, (year, month)) in entries {
			let time = zip::DateTime::from_date_and_time(*year, *month, 1, 0, 0, 0).unwrap();
			zip.start_file(*entry, zip::write::SimpleFileOptions::default().last_modified_time(time)).unwrap();
			zip.write_all(bytes).unwrap();
		}
		zip.finish().unwrap();
		self.archives.push(path);
		self
	}

	/// A loose file at `rel` under the install folder (`data/...` for the data folder),
	/// modified in the given year.
	pub(crate) fn loose(&mut self, rel: &str, bytes: &[u8], year: u16) -> &mut Game {
		let path = self.root.join(rel);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(&path, bytes).unwrap();
		let since_1970 = Duration::from_secs((u64::from(year) - 1970) * 365 * 86_400);
		std::fs::File::options().write(true).open(&path).unwrap().set_modified(SystemTime::UNIX_EPOCH + since_1970).unwrap();
		self
	}

	pub(crate) fn source(&self) -> DatScriptSource {
		DatScriptSource::open(&self.archives)
			.unwrap()
			.with_mounts(Mounts::new(vec![self.root.join("data")]).listed_at("data/", self.root.join("data")))
	}
}

impl Drop for Game {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.root);
	}
}

