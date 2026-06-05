//! Reading Battle Brothers `.dat` archives.
//!
//! `.dat` files are plain ZIP archives (all base-game entries STORED, no
//! compression). [`DatArchive`] opens one and reads entries by their archive path
//! (e.g. `scripts/ai/tactical/agent.cnut`). The mod-overlay / file-replacement
//! layer (multiple archives + loose mod `.zip`s, later entries shadowing earlier)
//! will build on top of this.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use zip::ZipArchive;

/// A single opened `.dat` (ZIP) archive.
pub struct DatArchive {
	archive: ZipArchive<File>,
}

impl DatArchive {
	/// Open the archive at `path`. Reads only the central directory up front;
	/// entries are read on demand (so a multi-GB `data_001.dat` is not loaded
	/// whole).
	pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
		let file = File::open(path)?;
		let archive = ZipArchive::new(file).map_err(zip_to_io)?;
		Ok(DatArchive { archive })
	}

	/// The full bytes of the entry at `name`, or `None` if there is no such entry.
	pub fn read(&mut self, name: &str) -> Option<Vec<u8>> {
		let mut entry = self.archive.by_name(name).ok()?;
		let mut bytes = Vec::with_capacity(entry.size() as usize);
		entry.read_to_end(&mut bytes).ok()?;
		Some(bytes)
	}

	/// Number of entries in the archive.
	pub fn len(&self) -> usize {
		self.archive.len()
	}

	/// Whether the archive has no entries.
	pub fn is_empty(&self) -> bool {
		self.archive.is_empty()
	}

	/// Every entry path in the archive.
	pub fn names(&self) -> Vec<String> {
		self.archive.file_names().map(str::to_owned).collect()
	}
}

fn zip_to_io(err: zip::result::ZipError) -> io::Error {
	io::Error::new(io::ErrorKind::InvalidData, err)
}
