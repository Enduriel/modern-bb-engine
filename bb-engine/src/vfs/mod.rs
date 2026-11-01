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

mod central_dir;
mod cp437;
mod dos_time;
mod entry_window;
mod index;
mod opened;
mod shared_file;

use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use zip::ZipArchive;

pub use entry_window::EntryWindow;
use index::{Entry, Index};
use shared_file::SharedFile;

/// A local file header's signature, and its fixed size.
const LOCAL: u32 = 0x0403_4b50;
const LOCAL_SIZE: u64 = 30;

/// A single opened `.dat` (ZIP) archive.
pub struct DatArchive {
	index: Arc<Index>,
	/// The archive's file: entries are read from it at their offsets.
	file: SharedFile,
	path: PathBuf,
	/// The `zip` crate's reading of the archive, for an entry whose sizes come only after
	/// its data (rare: a zip written as a stream), opened when one is first read.
	whole: Option<ZipArchive<SharedFile>>,
}

impl DatArchive {
	/// Open the archive at `path`. Reads only the central directory up front;
	/// entries are read on demand (so a multi-GB `data_001.dat` is not loaded
	/// whole).
	pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
		let path = path.as_ref().to_path_buf();
		let (index, file) = opened::open(&path)?;
		Ok(DatArchive { index, file, path, whole: None })
	}

	/// The path the archive was opened from.
	pub fn path(&self) -> &Path {
		&self.path
	}

	/// The entry at `name`, if it's there and can be read (not encrypted).
	fn entry(&self, name: &str) -> Option<Entry> {
		self.index.get(name).filter(|e| !e.encrypted()).copied()
	}

	/// The CRC32 and uncompressed size the central directory records for `name`, or
	/// `None` if there is no such entry. Reads no entry data.
	pub fn entry_meta(&self, name: &str) -> Option<(u32, u64)> {
		let entry = self.entry(name)?;
		Some((entry.crc32, entry.size))
	}

	/// When the entry at `name` was last modified, as Unix seconds: its directory's MS-DOS
	/// time read as local time, as the original reads it. `None` if there's no such entry.
	pub fn modified(&self, name: &str) -> Option<i64> {
		dos_time::to_unix(self.entry(name)?.modified)
	}

	/// The full bytes of the entry at `name`, or `None` if there is no such entry (or it
	/// doesn't read: its CRC is checked).
	pub fn read(&mut self, name: &str) -> Option<Vec<u8>> {
		let entry = self.entry(name)?;
		let mut bytes = Vec::with_capacity(entry.size as usize);
		if entry.sized_after() {
			if self.whole.is_none() {
				self.whole = Some(ZipArchive::new(self.file.clone()).ok()?);
			}
			self.whole.as_mut()?.by_name(name).ok()?.read_to_end(&mut bytes).ok()?;
			return Some(bytes);
		}
		// The `zip` crate reads the entry from its local header: its compression, its CRC.
		let mut file = self.file.clone();
		file.seek(SeekFrom::Start(entry.header)).ok()?;
		zip::read::read_zipfile_from_stream(&mut file).ok()??.read_to_end(&mut bytes).ok()?;
		Some(bytes)
	}

	/// Where the entry's data starts: after its local header and that header's name and
	/// extra field (which needn't match the directory's).
	fn data_start(&self, entry: &Entry) -> Option<u64> {
		let mut header = [0u8; LOCAL_SIZE as usize];
		let mut file = self.file.clone();
		file.seek(SeekFrom::Start(entry.header)).ok()?;
		file.read_exact(&mut header).ok()?;
		if u32::from_le_bytes(header[..4].try_into().ok()?) != LOCAL {
			return None;
		}
		let name = u64::from(u16::from_le_bytes([header[26], header[27]]));
		let extra = u64::from(u16::from_le_bytes([header[28], header[29]]));
		Some(entry.header + LOCAL_SIZE + name + extra)
	}

	/// The entry at `name` to read in place, if the archive stores it uncompressed (all of
	/// vanilla's entries; a mod's zip may compress its files). `None` if there's no such
	/// entry or it's compressed.
	pub fn stream(&mut self, name: &str) -> Option<EntryWindow> {
		let entry = self.entry(name).filter(|e| e.method == 0)?;
		Some(EntryWindow::new(self.file.clone(), self.data_start(&entry)?, entry.size))
	}

	/// Whether the archive has an entry at `name`.
	pub fn contains(&self, name: &str) -> bool {
		self.index.get(name).is_some()
	}

	/// Number of entries in the archive.
	pub fn len(&self) -> usize {
		self.index.len()
	}

	/// Whether the archive has no entries.
	pub fn is_empty(&self) -> bool {
		self.index.len() == 0
	}

	/// Every entry path in the archive.
	pub fn names(&self) -> Vec<String> {
		self.index.names().map(str::to_owned).collect()
	}
}
