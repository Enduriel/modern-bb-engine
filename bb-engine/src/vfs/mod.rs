//! Reading Battle Brothers `.dat` archives.
//!
//! `.dat` files are plain ZIP archives (all base-game entries STORED, no
//! compression). [`DatArchive`] opens one and reads entries by their archive path
//! (e.g. `scripts/ai/tactical/agent.cnut`). The mod-overlay / file-replacement
//! layer (multiple archives + loose mod `.zip`s, later entries shadowing earlier)
//! will build on top of this.
//!
//! An archive is parsed once per process ([`opened`]): every opener of the same file
//! shares its index.

mod entry_window;
mod opened;
mod shared_file;

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

pub use entry_window::EntryWindow;
use shared_file::SharedFile;

/// A single opened `.dat` (ZIP) archive.
pub struct DatArchive {
	archive: ZipArchive<SharedFile>,
	/// The archive's file, for reading an entry in place.
	file: SharedFile,
	path: PathBuf,
}

impl DatArchive {
	/// Open the archive at `path`. Reads only the central directory up front;
	/// entries are read on demand (so a multi-GB `data_001.dat` is not loaded
	/// whole).
	pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
		let path = path.as_ref().to_path_buf();
		let (archive, file) = opened::open(&path)?;
		Ok(DatArchive { archive, file, path })
	}

	/// The path the archive was opened from.
	pub fn path(&self) -> &Path {
		&self.path
	}

	/// The CRC32 and uncompressed size the central directory records for `name`, or
	/// `None` if there is no such entry. Reads no entry data.
	pub fn entry_meta(&mut self, name: &str) -> Option<(u32, u64)> {
		let entry = self.archive.by_name(name).ok()?;
		Some((entry.crc32(), entry.size()))
	}

	/// The full bytes of the entry at `name`, or `None` if there is no such entry.
	pub fn read(&mut self, name: &str) -> Option<Vec<u8>> {
		let mut entry = self.archive.by_name(name).ok()?;
		let mut bytes = Vec::with_capacity(entry.size() as usize);
		entry.read_to_end(&mut bytes).ok()?;
		Some(bytes)
	}

	/// The entry at `name` to read in place, if the archive stores it uncompressed (all of
	/// vanilla's entries; a mod's zip may compress its files). `None` if there's no such
	/// entry or it's compressed.
	pub fn stream(&mut self, name: &str) -> Option<EntryWindow> {
		let entry = self.archive.by_name(name).ok()?;
		// (An encrypted entry fails to open above.)
		if entry.compression() != zip::CompressionMethod::Stored {
			return None;
		}
		Some(EntryWindow::new(self.file.clone(), entry.data_start(), entry.size()))
	}

	/// Whether the archive has an entry at `name`.
	pub fn contains(&self, name: &str) -> bool {
		self.archive.index_for_name(name).is_some()
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
