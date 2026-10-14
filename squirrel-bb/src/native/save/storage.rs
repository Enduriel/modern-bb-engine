//! Where saves live, and the file container around a save.
//!
//! A save file is `MAGIC`, a `u32` format version, the metadata (see [`Meta`]), then
//! the body: the engine's world state interleaved with what the scripts wrote, in
//! save order. Saves go to a directory as `<name>.sav`, or, with no directory (tests,
//! tools), stay in memory.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::stream::{Meta, SaveReader, SaveWriter};

const MAGIC: &[u8; 8] = b"BBMESAVE";
/// Bumped whenever the engine-side layout changes.
pub(crate) const FORMAT_VERSION: u32 = 1;

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

/// A whole save file: header, metadata, then `body`.
pub(crate) fn encode(meta: &Meta, body: &[u8]) -> Vec<u8> {
	let mut w = SaveWriter::default();
	w.buf.extend_from_slice(MAGIC);
	w.u32(FORMAT_VERSION);
	meta.write(&mut w);
	w.buf.extend_from_slice(body);
	w.buf
}

/// Split a save file into its metadata and a reader positioned at the body.
pub(crate) fn decode(bytes: Vec<u8>) -> Result<(Meta, SaveReader), String> {
	if bytes.get(..MAGIC.len()) != Some(&MAGIC[..]) {
		return Err("not a save file of this engine".to_string());
	}
	let mut r = SaveReader::new(bytes[MAGIC.len()..].to_vec());
	let version = r.u32()?;
	if version != FORMAT_VERSION {
		return Err(format!("save format {version} is not supported (this engine writes {FORMAT_VERSION})"));
	}
	let meta = Meta::read(&mut r)?;
	Ok((meta, r))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn files_round_trip_through_memory() {
		let storage = Storage::Memory(RefCell::default());
		let meta = Meta {
			name: "Label".into(),
			file_name: "slot1".into(),
			..Meta::default()
		};
		storage.write("slot1", encode(&meta, &[1, 2, 3])).unwrap();
		assert_eq!(storage.names(), ["slot1"]);
		let (back, mut body) = decode(storage.read("slot1").unwrap()).unwrap();
		assert_eq!(back.name, "Label");
		assert_eq!((body.u8().unwrap(), body.u8().unwrap(), body.u8().unwrap()), (1, 2, 3));
		storage.delete("slot1");
		assert!(storage.names().is_empty());
	}

	#[test]
	fn rejects_foreign_files() {
		assert!(decode(b"not a save".to_vec()).is_err());
	}
}
