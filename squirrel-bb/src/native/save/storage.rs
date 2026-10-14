//! Where saves live, and the file container around a save.
//!
//! A save file of this engine's own format is `MAGIC`, a `u32` format version, the
//! metadata (see [`Meta`]), then the body: the engine's world state interleaved with
//! what the scripts wrote, in save order. Files in the original's container (its saves,
//! and the storages `PersistenceManager` writes) are read by [`decode`] too. Saves go to
//! a directory as `<name>.sav`, or, with no directory (tests, tools), stay in memory.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::container;
use super::stream::{Meta, SaveReader, SaveWriter};

const MAGIC: &[u8; 8] = b"BBMESAVE";
/// Bumped whenever the engine-side layout changes.
pub(crate) const FORMAT_VERSION: u32 = 4;

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
				std::fs::write(Self::path(dir, name), bytes).map_err(|e| format!("writing save {name}: {e}"))
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

	#[test]
	fn rejects_foreign_files() {
		assert!(decode(b"not a save".to_vec(), "x").is_err());
	}
}
