//! Where saves live, and the file container around a save.
//!
//! A save file of this engine's own format is `MAGIC`, a `u32` format version, the
//! metadata (see [`Meta`]), then the body: the engine's world state interleaved with
//! what the scripts wrote, in save order. Files in the original's container (its saves,
//! and the storages `PersistenceManager` writes) are read by [`decode`] too. Saves go to
//! a directory as `<name>.sav`, or, with no directory (tests, tools), stay in memory.
//!
//! A save is written whole or not at all: into a temporary file next to it, flushed to
//! the disk, then renamed over the old one (and the folder flushed), so a crash or a
//! full disk mid-write leaves the old save as it was. (The original writes in place: a
//! save interrupted while the game was in trouble came out corrupt.)

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::container;
use super::stream::{Meta, SaveReader, SaveWriter};

const MAGIC: &[u8; 8] = b"BBMESAVE";
/// Bumped whenever the engine-side layout changes.
pub(crate) const FORMAT_VERSION: u32 = 5;

/// The save store.
pub(crate) enum Storage {
	Memory(RefCell<BTreeMap<String, Vec<u8>>>),
	Dir(PathBuf),
}

impl Storage {
	fn path(dir: &std::path::Path, name: &str) -> PathBuf {
		dir.join(format!("{name}.sav"))
	}

	pub fn write(&self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
		match self {
			Storage::Memory(m) => {
				m.borrow_mut().insert(name.to_string(), bytes);
				Ok(())
			}
			Storage::Dir(dir) => {
				std::fs::create_dir_all(dir).map_err(|e| format!("save directory: {e}"))?;
				let temp = write_temp(dir, name, &bytes).map_err(|e| format!("writing save {name}: {e}"))?;
				commit(dir, &temp, &Self::path(dir, name)).map_err(|e| format!("writing save {name}: {e}"))
			}
		}
	}

	pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
		match self {
			Storage::Memory(m) => m.borrow().get(name).cloned().ok_or_else(|| format!("no save named {name}")),
			Storage::Dir(dir) => std::fs::read(Self::path(dir, name)).map_err(|e| format!("reading save {name}: {e}")),
		}
	}

	/// Whether save `name` exists.
	pub fn exists(&self, name: &str) -> bool {
		match self {
			Storage::Memory(m) => m.borrow().contains_key(name),
			Storage::Dir(dir) => Self::path(dir, name).is_file(),
		}
	}

	pub fn delete(&self, name: &str) {
		match self {
			Storage::Memory(m) => {
				m.borrow_mut().remove(name);
			}
			Storage::Dir(dir) => {
				let _ = std::fs::remove_file(Self::path(dir, name));
			}
		}
	}

	/// The bytes of the saves kept in memory (none for a directory).
	pub fn memory_bytes(&self) -> usize {
		match self {
			Storage::Memory(m) => m.borrow().values().map(Vec::len).sum(),
			Storage::Dir(_) => 0,
		}
	}

	/// The names of every stored save.
	pub fn names(&self) -> Vec<String> {
		match self {
			Storage::Memory(m) => m.borrow().keys().cloned().collect(),
			Storage::Dir(dir) => {
				let Ok(entries) = std::fs::read_dir(dir) else {
					return Vec::new();
				};
				let mut names: Vec<String> = entries
					.flatten()
					.filter_map(|e| e.file_name().to_str()?.strip_suffix(".sav").map(str::to_string))
					.collect();
				names.sort();
				names
			}
		}
	}
}

/// Save `name`'s bytes in a temporary file in `dir` (not a `.sav`, so not listed),
/// flushed to the disk; its path.
fn write_temp(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
	use std::io::Write;
	let temp = dir.join(format!("{name}.sav.writing"));
	let mut file = std::fs::File::create(&temp)?;
	file.write_all(bytes)?;
	file.sync_all()?;
	Ok(temp)
}

/// Put the written `temp` in place of `path` at once, and flush the folder (where the
/// system can) so the rename itself is on the disk.
fn commit(dir: &std::path::Path, temp: &std::path::Path, path: &std::path::Path) -> std::io::Result<()> {
	std::fs::rename(temp, path)?;
	#[cfg(unix)]
	std::fs::File::open(dir)?.sync_all()?;
	#[cfg(not(unix))]
	let _ = dir;
	Ok(())
}

/// Which format a file is in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Format {
	/// This engine's own.
	Own,
	/// The original's container (and, for a campaign save, its body layout).
	Original,
}

/// A whole save file of this engine's own format: header, metadata, then `body`.
pub(crate) fn encode(meta: &Meta, body: &[u8]) -> Vec<u8> {
	let mut w = SaveWriter::default();
	w.buf.extend_from_slice(MAGIC);
	w.u32(FORMAT_VERSION);
	w.i64(meta.version);
	w.str(&meta.name);
	w.str(&meta.file_name);
	w.i64(meta.created);
	w.i64(meta.modified);
	w.u32(meta.data.len() as u32);
	for (k, v) in &meta.data {
		w.str(k);
		w.str(v);
	}
	w.buf.extend_from_slice(body);
	w.buf
}

/// Split save file `bytes` (stored as `name`), in either format, into its format, its
/// metadata and a reader positioned at the body.
pub(crate) fn decode(bytes: Vec<u8>, name: &str) -> Result<(Format, Meta, SaveReader), String> {
	if container::is_original(&bytes) {
		let (meta, r) = container::decode(bytes, name)?;
		return Ok((Format::Original, meta, r));
	}
	if bytes.get(..MAGIC.len()) != Some(&MAGIC[..]) {
		return Err(format!("{name}: not a save file"));
	}
	let mut r = SaveReader::new(bytes);
	r.seek(MAGIC.len());
	let version = r.u32()?;
	if version != FORMAT_VERSION {
		return Err(format!("save format {version} is not supported (this engine writes {FORMAT_VERSION})"));
	}
	let mut meta = Meta {
		version: r.i64()?,
		name: r.str()?,
		file_name: r.str()?,
		created: r.i64()?,
		modified: r.i64()?,
		..Meta::default()
	};
	for _ in 0..r.u32()? {
		let k = r.str()?;
		meta.data.insert(k, r.str()?);
	}
	Ok((Format::Own, meta, r))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn files_round_trip_through_memory() {
		let storage = Storage::Memory(RefCell::default());
		let mut meta = Meta {
			version: 72,
			name: "Label".into(),
			file_name: "slot1".into(),
			created: 1_760_000_000,
			modified: 1_760_000_001,
			..Meta::default()
		};
		meta.set_int("days".into(), 12);
		meta.set_string("banner".into(), "banner_04".into());
		storage.write("slot1", encode(&meta, &[1, 2, 3])).unwrap();
		assert_eq!(storage.names(), ["slot1"]);
		let (format, back, mut body) = decode(storage.read("slot1").unwrap(), "slot1").unwrap();
		assert_eq!(format, Format::Own);
		assert_eq!(back, meta);
		assert_eq!((body.u8().unwrap(), body.u8().unwrap(), body.u8().unwrap()), (1, 2, 3));
		storage.delete("slot1");
		assert!(storage.names().is_empty());
	}

	/// A fresh, empty folder for one test, removed again by `Drop`.
	struct Folder(PathBuf);

	impl Folder {
		fn new(test: &str) -> Folder {
			let dir = std::env::temp_dir().join(format!("bb-saves-{test}-{}", std::process::id()));
			let _ = std::fs::remove_dir_all(&dir);
			Folder(dir)
		}
	}

	impl Drop for Folder {
		fn drop(&mut self) {
			let _ = std::fs::remove_dir_all(&self.0);
		}
	}

	#[test]
	fn saves_in_a_folder_are_written_whole_and_replaced_whole() {
		let folder = Folder::new("whole");
		let storage = Storage::Dir(folder.0.clone());
		storage.write("slot1", b"first".to_vec()).unwrap();
		assert_eq!(storage.read("slot1").unwrap(), b"first");
		storage.write("slot1", b"second".to_vec()).unwrap();
		assert_eq!(storage.read("slot1").unwrap(), b"second");
		assert_eq!(storage.names(), ["slot1"]);
		// Nothing is left of the writing.
		let files: Vec<_> = std::fs::read_dir(&folder.0).unwrap().flatten().map(|e| e.file_name()).collect();
		assert_eq!(files, ["slot1.sav"]);
		storage.delete("slot1");
		assert!(storage.names().is_empty() && !storage.exists("slot1"));
	}

	#[test]
	fn a_save_cut_short_leaves_the_old_one() {
		let folder = Folder::new("cut");
		let storage = Storage::Dir(folder.0.clone());
		storage.write("slot1", b"the old save".to_vec()).unwrap();
		// A crash after writing the new bytes, before they're put in place.
		write_temp(&folder.0, "slot1", b"the new sa").unwrap();
		assert_eq!(storage.read("slot1").unwrap(), b"the old save");
		assert_eq!(storage.names(), ["slot1"], "the leftover isn't a save");
		// The next save goes through over the leftover.
		storage.write("slot1", b"the new save".to_vec()).unwrap();
		assert_eq!(storage.read("slot1").unwrap(), b"the new save");
	}

	#[test]
	fn rejects_foreign_files() {
		assert!(decode(b"not a save".to_vec(), "x").is_err());
	}
}
