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
pub(crate) const FORMAT_VERSION: u32 = 7;

/// How much of a save [`Storage::read_meta`] reads first: a page, where the original's
/// saves keep their metadata (about 200 bytes vanilla's, 1.7 KB the most among 119 saves
/// of a modded install).
const HEAD: usize = 4096;

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
				commit(dir, &temp, &Self::path(dir, name)).map_err(|e| {
					let _ = std::fs::remove_file(&temp);
					format!("writing save {name}: {e}")
				})
			}
		}
	}

	pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
		match self {
			Storage::Memory(m) => m.borrow().get(name).cloned().ok_or_else(|| format!("no save named {name}")),
			Storage::Dir(dir) => std::fs::read(Self::path(dir, name)).map_err(|e| format!("reading save {name}: {e}")),
		}
	}

	/// At most the first `limit` bytes of save `name` (all of them if it's shorter).
	fn read_head(&self, name: &str, limit: usize) -> Result<Vec<u8>, String> {
		match self {
			Storage::Memory(m) => {
				let saves = m.borrow();
				let bytes = saves.get(name).ok_or_else(|| format!("no save named {name}"))?;
				Ok(bytes[..bytes.len().min(limit)].to_vec())
			}
			Storage::Dir(dir) => {
				use std::io::Read;
				let mut head = Vec::with_capacity(limit);
				std::fs::File::open(Self::path(dir, name))
					.and_then(|f| f.take(limit as u64).read_to_end(&mut head))
					.map_err(|e| format!("reading save {name}: {e}"))?;
				Ok(head)
			}
		}
	}

	/// Save `name`'s format and metadata, as [`decode`] gives them, read from the start of
	/// the file: the load menu lists every save's, and reading whole saves (about 7 MB
	/// each) for it took 0.4 to 0.6 s for 120 in bb, where this takes 1 ms (25 ms with
	/// none of them cached; measured 2026-10-22; the original's took 0.14 s for 130,
	/// cached). A header that doesn't fit in [`HEAD`]
	/// (or a file that isn't a save) is read again whole.
	pub fn read_meta(&self, name: &str) -> Result<(Format, Meta), String> {
		let head = self.read_head(name, HEAD)?;
		let whole = head.len() < HEAD;
		match decode(head, name) {
			Ok((format, meta, _)) => Ok((format, meta)),
			Err(e) if whole => Err(e),
			Err(_) => decode(self.read(name)?, name).map(|(format, meta, _)| (format, meta)),
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

	/// The names of every stored save, in the original's order: its PhysFS lists a
	/// folder sorted by `strcmp` of the whole file names (`<name>.sav`, byte by byte,
	/// case-sensitive), whatever order the file system gives them in.
	pub fn names(&self) -> Vec<String> {
		let mut names: Vec<String> = match self {
			Storage::Memory(m) => m.borrow().keys().cloned().collect(),
			Storage::Dir(dir) => {
				let Ok(entries) = std::fs::read_dir(dir) else {
					return Vec::new();
				};
				entries
					.flatten()
					.filter_map(|e| e.file_name().to_str()?.strip_suffix(".sav").map(str::to_string))
					.collect()
			}
		};
		names.sort_by_cached_key(|n| format!("{n}.sav"));
		names
	}
}

/// Save `name`'s bytes in a temporary file in `dir` (not a `.sav`, so not listed),
/// flushed to the disk; its path. The file is this write's own: two processes saving
/// under one name at once (two runs sharing a savegames folder) would otherwise write
/// into one file, and publish a mix of both saves or fail to find it to rename.
fn write_temp(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
	use std::io::Write;
	use std::sync::atomic::{AtomicU64, Ordering};
	static WRITES: AtomicU64 = AtomicU64::new(0);
	let n = WRITES.fetch_add(1, Ordering::Relaxed);
	let temp = dir.join(format!("{name}.sav.{}-{n}.writing", std::process::id()));
	let written = (|| {
		let mut file = std::fs::File::create_new(&temp)?;
		file.write_all(bytes)?;
		file.sync_all()
	})();
	if let Err(e) = written {
		let _ = std::fs::remove_file(&temp);
		return Err(e);
	}
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
			file_name: "bb_probe_slot1".into(),
			created: 1_760_000_000,
			modified: 1_760_000_001,
			..Meta::default()
		};
		meta.set_int("days".into(), 12);
		meta.set_string("banner".into(), "banner_04".into());
		storage.write("bb_probe_slot1", encode(&meta, &[1, 2, 3])).unwrap();
		assert_eq!(storage.names(), ["bb_probe_slot1"]);
		let (format, back, mut body) = decode(storage.read("bb_probe_slot1").unwrap(), "bb_probe_slot1").unwrap();
		assert_eq!(format, Format::Own);
		assert_eq!(back, meta);
		assert_eq!((body.u8().unwrap(), body.u8().unwrap(), body.u8().unwrap()), (1, 2, 3));
		storage.delete("bb_probe_slot1");
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
		storage.write("bb_probe_slot1", b"first".to_vec()).unwrap();
		assert_eq!(storage.read("bb_probe_slot1").unwrap(), b"first");
		storage.write("bb_probe_slot1", b"second".to_vec()).unwrap();
		assert_eq!(storage.read("bb_probe_slot1").unwrap(), b"second");
		assert_eq!(storage.names(), ["bb_probe_slot1"]);
		// Nothing is left of the writing.
		let files: Vec<_> = std::fs::read_dir(&folder.0).unwrap().flatten().map(|e| e.file_name()).collect();
		assert_eq!(files, ["bb_probe_slot1.sav"]);
		storage.delete("bb_probe_slot1");
		assert!(storage.names().is_empty() && !storage.exists("bb_probe_slot1"));
	}

	#[test]
	fn a_save_cut_short_leaves_the_old_one() {
		let folder = Folder::new("cut");
		let storage = Storage::Dir(folder.0.clone());
		storage.write("bb_probe_slot1", b"the old save".to_vec()).unwrap();
		// A crash after writing the new bytes, before they're put in place.
		write_temp(&folder.0, "bb_probe_slot1", b"the new sa").unwrap();
		assert_eq!(storage.read("bb_probe_slot1").unwrap(), b"the old save");
		assert_eq!(storage.names(), ["bb_probe_slot1"], "the leftover isn't a save");
		// The next save goes through over the leftover.
		storage.write("bb_probe_slot1", b"the new save".to_vec()).unwrap();
		assert_eq!(storage.read("bb_probe_slot1").unwrap(), b"the new save");
	}

	#[test]
	fn writers_saving_one_name_at_once_each_publish_a_whole_save() {
		// Two runs sharing a savegames folder (here: threads) once wrote into one temporary
		// file: mixed saves, or a rename that found no file.
		let folder = Folder::new("concurrent");
		std::fs::create_dir_all(&folder.0).unwrap();
		let payload = |w: u8| vec![w; 256 * 1024];
		std::thread::scope(|scope| {
			for w in 0..4u8 {
				let dir = folder.0.clone();
				scope.spawn(move || {
					let storage = Storage::Dir(dir);
					for _ in 0..10 {
						storage.write("bb_probe_slot1", payload(w)).unwrap();
						let back = Storage::Dir(storage_dir(&storage)).read("bb_probe_slot1").unwrap();
						assert!(back.iter().all(|&b| b == back[0]) && back.len() == 256 * 1024, "a mixed save");
					}
				});
			}
		});
		let files: Vec<_> = std::fs::read_dir(&folder.0).unwrap().flatten().map(|e| e.file_name()).collect();
		assert_eq!(files, ["bb_probe_slot1.sav"]);
	}

	fn storage_dir(storage: &Storage) -> PathBuf {
		match storage {
			Storage::Dir(dir) => dir.clone(),
			Storage::Memory(_) => unreachable!(),
		}
	}

	/// The load menu's metadata comes from the start of each file, whatever its format,
	/// its body or the size of its header, and is what decoding the whole file gives.
	#[test]
	fn metadata_is_read_from_the_start_of_the_file() {
		let folder = Folder::new("meta");
		let storage = Storage::Dir(folder.0.clone());
		let mut meta = Meta {
			version: 72,
			name: "Label".into(),
			file_name: "bb_probe_big".into(),
			created: 1_760_000_000,
			modified: 1_760_000_001,
			..Meta::default()
		};
		meta.set_string("groupname".into(), "Probe Company".into());
		let body = vec![7u8; 3 << 20];
		storage.write("bb_probe_big", container::encode(&meta, &body)).unwrap();
		// A header past the first read (a mod's long metadata) is read whole.
		let mut long = meta.clone();
		long.file_name = "bb_probe_long".into();
		long.set_string("notes".into(), "x".repeat(3 * HEAD));
		storage.write("bb_probe_long", container::encode(&long, &body)).unwrap();
		// This engine's own format, shorter than the first read.
		let mut own = meta.clone();
		own.file_name = "bb_probe_own".into();
		storage.write("bb_probe_own", encode(&own, &[1, 2, 3])).unwrap();
		storage.write("bb_probe_broken", container::encode(&meta, &[])[..20].to_vec()).unwrap();
		storage.write("bb_probe_junk", vec![0; 2 * HEAD]).unwrap();
		for name in ["bb_probe_big", "bb_probe_long", "bb_probe_own"] {
			let (format, whole, _) = decode(storage.read(name).unwrap(), name).unwrap();
			assert_eq!(storage.read_meta(name).unwrap(), (format, whole), "{name}");
		}
		assert_eq!(storage.read_meta("bb_probe_long").unwrap().1.get_string("notes").len(), 3 * HEAD);
		assert_eq!(storage.read_meta("bb_probe_own").unwrap().0, Format::Own);
		for name in ["bb_probe_broken", "bb_probe_junk", "bb_probe_missing"] {
			assert!(storage.read_meta(name).is_err(), "{name}");
		}
	}

	#[test]
	fn rejects_foreign_files() {
		assert!(decode(b"not a save".to_vec(), "x").is_err());
	}
}
