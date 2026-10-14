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
//!
//! Directories can be mounted beside them ([`Mounts`]: the install and data folders, as
//! in the original). A path no archive provides is looked up there. Which of the two
//! wins for a path both provide is unverified; the archives do here.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;

use squirrel_bb::{LoadedScript, ScriptSource};

use crate::mounts::Mounts;
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
	mounts: Mounts,
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
			mounts: Mounts::default(),
		})
	}

	/// Mount directories beside the archives (see [`Mounts`]).
	pub fn with_mounts(mut self, mounts: Mounts) -> Self {
		self.mounts = mounts;
		self
	}

	/// A script from the mounted directories: a loose `.nut`, else a loose `.cnut`.
	fn load_mounted(&self, path: &str) -> Option<LoadedScript> {
		if let Some(source) = self.mounts.read(&format!("{path}.nut")) {
			return Some(LoadedScript::Source(source));
		}
		let encrypted = self.mounts.read(&format!("{path}.cnut"))?;
		transcode_cnut(&encrypted).ok().map(LoadedScript::Bytecode)
	}

	/// Run `f` over the opened archives, in load order (e.g. to load the brushes).
	pub fn with_archives<R>(&self, f: impl FnOnce(&mut [DatArchive]) -> R) -> R {
		f(&mut self.archives.borrow_mut())
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
			.or_else(|| self.load_mounted(path))
	}

	fn caches_compiles(&self) -> bool {
		self.cache.is_some()
	}

	fn load_for(&self, path: &str, context: &dyn Fn() -> u64) -> Option<LoadedScript> {
		let mut archives = self.archives.borrow_mut();
		self.load_nut(&mut archives, path, Some(context))
			.or_else(|| self.load_cnut(&mut archives, path))
			.or_else(|| self.load_mounted(path))
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
		archives
			.iter_mut()
			.rev()
			.find_map(|archive| archive.read(path))
			.or_else(|| self.mounts.read(path))
	}

	fn list_prefix(&self, prefix: &str) -> Option<Vec<String>> {
		let dir = if prefix.is_empty() || prefix.ends_with('/') {
			prefix.to_string()
		} else {
			format!("{prefix}/")
		};
		let archives = self.archives.borrow();
		// Every file, not just scripts (mods list their JS under `ui/mods/`), without
		// its extension, as the original lists them. A BTreeSet sorts and dedupes (a
		// script as both .nut and .cnut, the hex masks `.0`–`.3`, a path in several
		// archives or mounts).
		let mut paths = BTreeSet::new();
		for archive in archives.iter() {
			for name in archive.names() {
				// `dir` ends in '/', so this matches at a segment boundary.
				if name.starts_with(&dir) && !name.ends_with('/') {
					paths.insert(without_extension(&name).to_string());
				}
			}
		}
		for name in self.mounts.files_under(&dir) {
			paths.insert(without_extension(&name).to_string());
		}
		if paths.is_empty() {
			None
		} else {
			Some(paths.into_iter().collect())
		}
	}
}

/// `path` without its file name's extension (from the last `.`), as `IO.enumerateFiles`
/// lists files: `scripts/a.nut` is `scripts/a`, `gfx/masks/hex_mask.0` is
/// `gfx/masks/hex_mask`.
fn without_extension(path: &str) -> &str {
	let name_at = path.rfind('/').map_or(0, |i| i + 1);
	match path[name_at..].rfind('.') {
		Some(dot) => &path[..name_at + dot],
		None => path,
	}
}
