//! A [`ScriptSource`] over the game's `.dat` archives.
//!
//! For a script path it prefers a `.nut` (source) — a mod overriding a vanilla
//! file ships source — and otherwise falls back to the vanilla `.cnut` (compiled),
//! which is decrypted + transcoded to our VM's bytecode on the fly.
//!
//! With a [`ScriptCache`], a `.cnut`'s transcoded bytecode is kept on disk, keyed on
//! the archive entry (path, CRC32, size), so a warm start skips decrypt + transcode.
//!
//! Archives are passed in **load order** (ascending by filename, the BB rule:
//! `data_*` then `mod_*`, since `'d' < 'm'`), and a later archive **overrides** an
//! earlier one — so resolution scans them in reverse and takes the first hit.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;

use squirrel_bb::{LoadedScript, ScriptSource};

use crate::script_cache::{CacheKey, ScriptCache};
use crate::transcode::transcode_cnut;
use crate::vfs::DatArchive;

/// A script source backed by one or more opened `.dat` archives.
pub struct DatScriptSource {
	// `RefCell` because `ScriptSource::load` is `&self` but reading a zip entry
	// needs `&mut` (it seeks). The module system never holds a borrow across the
	// VM calls that would re-enter, so this is single-threaded and conflict-free.
	archives: RefCell<Vec<DatArchive>>,
	cache: Option<ScriptCache>,
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
			cache: None,
		})
	}

	/// Keep scripts in `cache` (see [`ScriptCache`]): transcoded `.cnut`s, and `.nut`s
	/// compiled by the module system.
	pub fn with_cache(mut self, cache: Option<ScriptCache>) -> Self {
		self.cache = cache;
		self
	}

	/// The `.nut` source for `path` (the last archive providing one wins). With a cache
	/// and a compile `context`, bytecode an identical earlier compile left there instead.
	fn load_nut(
		&self,
		archives: &mut [DatArchive],
		path: &str,
		context: Option<&dyn Fn() -> u64>,
	) -> Option<LoadedScript> {
		let nut = format!("{path}.nut");
		for archive in archives.iter_mut().rev() {
			if let (Some(cache), Some(context)) = (&self.cache, context)
				&& let Some((crc32, size)) = archive.entry_meta(&nut)
			{
				let archive_path = archive.path().to_path_buf();
				let key = CacheKey {
					kind: "nut",
					archive: &archive_path,
					entry: &nut,
					crc32,
					size,
					context: context(),
				};
				if let Some(bytecode) = cache.get(&key) {
					return Some(LoadedScript::Bytecode(bytecode));
				}
			}
			if let Some(bytes) = archive.read(&nut) {
				return Some(LoadedScript::Source(bytes));
			}
		}
		None
	}

	/// The vanilla compiled `.cnut` for `path`, transcoded to our format (through the
	/// cache, if any).
	fn load_cnut(&self, archives: &mut [DatArchive], path: &str) -> Option<LoadedScript> {
		let cnut = format!("{path}.cnut");
		for archive in archives.iter_mut().rev() {
			let Some(cache) = &self.cache else {
				if let Some(encrypted) = archive.read(&cnut) {
					return transcode_cnut(&encrypted).ok().map(LoadedScript::Bytecode);
				}
				continue;
			};
			let Some((crc32, size)) = archive.entry_meta(&cnut) else {
				continue;
			};
			let archive_path = archive.path().to_path_buf();
			let key = CacheKey {
				kind: "cnut",
				archive: &archive_path,
				entry: &cnut,
				crc32,
				size,
				context: 0,
			};
			if let Some(bytecode) = cache.get(&key) {
				return Some(LoadedScript::Bytecode(bytecode));
			}
			if let Some(encrypted) = archive.read(&cnut) {
				let bytecode = transcode_cnut(&encrypted).ok()?;
				cache.put(&key, &bytecode);
				return Some(LoadedScript::Bytecode(bytecode));
			}
		}
		None
	}
}

impl ScriptSource for DatScriptSource {
	// Scan in reverse (last-loaded archive wins). A `.nut` (typically a mod's source)
	// anywhere is preferred over any compiled `.cnut`.
	fn load(&self, path: &str) -> Option<LoadedScript> {
		let mut archives = self.archives.borrow_mut();
		self.load_nut(&mut archives, path, None)
			.or_else(|| self.load_cnut(&mut archives, path))
	}

	fn caches_compiles(&self) -> bool {
		self.cache.is_some()
	}

	fn load_for(&self, path: &str, context: &dyn Fn() -> u64) -> Option<LoadedScript> {
		let mut archives = self.archives.borrow_mut();
		self.load_nut(&mut archives, path, Some(context))
			.or_else(|| self.load_cnut(&mut archives, path))
	}

	fn compiled(&self, path: &str, context: u64, bytecode: &[u8]) {
		let Some(cache) = &self.cache else {
			return;
		};
		let nut = format!("{path}.nut");
		let mut archives = self.archives.borrow_mut();
		let Some((archive, (crc32, size))) = archives.iter_mut().rev().find_map(|a| {
			a.entry_meta(&nut)
				.map(|meta| (a.path().to_path_buf(), meta))
		}) else {
			return;
		};
		let key = CacheKey {
			kind: "nut",
			archive: &archive,
			entry: &nut,
			crc32,
			size,
			context,
		};
		cache.put(&key, bytecode);
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
