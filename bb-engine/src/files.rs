//! [`GameFiles`]: the game's non-script files across the mounted archives, readable
//! from any thread.
//!
//! The renderer's texture loader reads atlases on worker threads, so this holds each
//! archive behind its own lock (reading a zip entry seeks the file). A path resolves
//! like a script: the last archive that has it wins, so a mod's copy replaces
//! vanilla's.

use std::io;
use std::path::Path;
use std::sync::Mutex;

use bb_assets::FileSource;

use crate::vfs::DatArchive;

/// The mounted archives, in load order.
pub struct GameFiles {
	archives: Vec<Mutex<DatArchive>>,
}

impl GameFiles {
	/// Open the archives at `paths`, in load order (later ones override earlier ones).
	pub fn open(paths: impl IntoIterator<Item = impl AsRef<Path>>) -> io::Result<Self> {
		let archives = paths
			.into_iter()
			.map(|path| DatArchive::open(path).map(Mutex::new))
			.collect::<io::Result<_>>()?;
		Ok(GameFiles { archives })
	}

	/// The bytes of the file at `path` from the last archive that has it.
	pub fn read(&self, path: &str) -> Option<Vec<u8>> {
		self.archives
			.iter()
			.rev()
			.find_map(|archive| archive.lock().unwrap().read(path))
	}

	/// Every entry path in every archive (duplicates included).
	pub fn names(&self) -> Vec<String> {
		self.archives
			.iter()
			.flat_map(|archive| archive.lock().unwrap().names())
			.collect()
	}
}

impl FileSource for GameFiles {
	fn read(&self, path: &str) -> Option<Vec<u8>> {
		GameFiles::read(self, path)
	}
}
