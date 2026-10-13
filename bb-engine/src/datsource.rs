//! A [`ScriptSource`] over the game's `.dat` archives.
//!
//! For a script path it prefers a `.nut` (source) — a mod overriding a vanilla
//! file ships source — and otherwise falls back to the vanilla `.cnut` (compiled),
//! which is decrypted + transcoded to our VM's bytecode on the fly.
//!
//! Archives are passed in **load order** (ascending by filename, the BB rule:
//! `data_*` then `mod_*`, since `'d' < 'm'`), and a later archive **overrides** an
//! earlier one — so resolution scans them in reverse and takes the first hit.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;

use squirrel_bb::{LoadedScript, ScriptSource};

use crate::transcode::transcode_cnut;
use crate::vfs::DatArchive;

/// A script source backed by one or more opened `.dat` archives.
pub struct DatScriptSource {
	// `RefCell` because `ScriptSource::load` is `&self` but reading a zip entry
	// needs `&mut` (it seeks). The module system never holds a borrow across the
	// VM calls that would re-enter, so this is single-threaded and conflict-free.
	archives: RefCell<Vec<DatArchive>>,
}

impl DatScriptSource {
	/// Open the given archives, in load order (ascending by filename; later
	/// overrides earlier).
	pub fn open(paths: impl IntoIterator<Item = impl AsRef<Path>>) -> std::io::Result<Self> {
		let mut archives = Vec::new();
		for path in paths {
			archives.push(DatArchive::open(path)?);
		}
		Ok(DatScriptSource {
			archives: RefCell::new(archives),
		})
	}
}

impl ScriptSource for DatScriptSource {
	fn load(&self, path: &str) -> Option<LoadedScript> {
		let mut archives = self.archives.borrow_mut();

		// Scan in reverse (last-loaded archive wins). A `.nut` (typically a mod's
		// source) anywhere is preferred over any compiled `.cnut`.
		let nut = format!("{path}.nut");
		for archive in archives.iter_mut().rev() {
			if let Some(bytes) = archive.read(&nut) {
				return Some(LoadedScript::Source(bytes));
			}
		}

		// Otherwise the vanilla compiled `.cnut`, transcoded to our format.
		let cnut = format!("{path}.cnut");
		for archive in archives.iter_mut().rev() {
			if let Some(encrypted) = archive.read(&cnut) {
				return transcode_cnut(&encrypted).ok().map(LoadedScript::Bytecode);
			}
		}
		None
	}

	fn read_file(&self, path: &str) -> Option<Vec<u8>> {
		let mut archives = self.archives.borrow_mut();
		archives.iter_mut().rev().find_map(|archive| archive.read(path))
	}

	fn list_prefix(&self, prefix: &str) -> Option<Vec<String>> {
		let dir = if prefix.is_empty() || prefix.ends_with('/') {
			prefix.to_string()
		} else {
			format!("{prefix}/")
		};
		let archives = self.archives.borrow();
		// BTreeSet: sorted + deduped (a path present as both .nut and .cnut, or in
		// several archives, lists once).
		let mut paths = BTreeSet::new();
		for archive in archives.iter() {
			for name in archive.names() {
				if let Some(rest) = name.strip_prefix(&dir) {
					// (`strip_prefix` of `dir` ensures the segment boundary; `dir`
					// ends in '/'.) Keep only script files.
					let _ = rest;
					if let Some(stripped) = name
						.strip_suffix(".nut")
						.or_else(|| name.strip_suffix(".cnut"))
					{
						paths.insert(stripped.to_string());
					}
				}
			}
		}
		if paths.is_empty() {
			None
		} else {
			Some(paths.into_iter().collect())
		}
	}
}
