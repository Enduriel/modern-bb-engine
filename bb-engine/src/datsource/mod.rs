//! A [`ScriptSource`] over the game's `.dat` archives.
//!
//! A path resolves as the original's search path has it (probed 2026-11-01): the data
//! folder's loose files first, then the archives. (The original then looks in the install
//! folder; we don't read it, see [`Mounts`].)
//! Archives are mounted in the original's **load order** ([`load_order`]: every archive
//! by file name, so `data_*` before `mod_*`), and a later archive **overrides** an
//! earlier one, so they are searched in reverse.
//!
//! A script is a `.nut` (source) or a `.cnut` (compiled, decrypted + transcoded to our
//! VM's bytecode on the fly). Each resolves on its own, and when both exist the newer
//! wins, a tie going to the `.cnut` ([`source_is_newer`]): a mod's `.nut` replaces a
//! vanilla script only when its time is after the `.cnut`'s.
//!
//! With a [`ScriptCache`], a `.cnut`'s transcoded bytecode is kept on disk, keyed on
//! the archive entry (path, CRC32, size), so a warm start skips decrypt + transcode.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use squirrel_bb::{LoadedScript, ScriptSource};

use crate::load_order;
use crate::mounts::{self, Mounts};
use crate::script_cache::{CacheKey, ScriptCache};
use crate::transcode::transcode_cnut;
use crate::vfs::DatArchive;

/// Whether a script's `.nut`, last modified at `nut` (Unix seconds), is loaded over its
/// `.cnut`, last modified at `cnut`: only when it is newer (probed 2026-11-01: equal
/// times load the `.cnut`; a time that can't be read counts as -1, as PhysFS has it).
pub fn source_is_newer(nut: Option<i64>, cnut: Option<i64>) -> bool {
	nut.unwrap_or(-1) > cnut.unwrap_or(-1)
}

/// Where a file resolves: a loose file in a mounted directory, or an archive (by its
/// index in load order).
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
	Dir(PathBuf),
	Archive(usize),
}

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
	/// Open the given archives, mounted in the original's load order ([`load_order`]: by
	/// file name; a later one overrides an earlier one).
	pub fn open(paths: impl IntoIterator<Item = impl AsRef<Path>>) -> std::io::Result<Self> {
		let mut paths: Vec<_> = paths.into_iter().collect();
		load_order::sort(&mut paths);
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

	/// The mounted directories.
	pub fn mounts(&self) -> &Mounts {
		&self.mounts
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

	/// Where the file `name` (with its extension) resolves: the data folder, then the
	/// archives from the last.
	fn locate(&self, archives: &[DatArchive], name: &str) -> Option<Place> {
		self.mounts
			.find(name)
			.map(Place::Dir)
			.or_else(|| archives.iter().rposition(|a| a.contains(name)).map(Place::Archive))
	}

	/// When the file `name` at `place` was last modified, as Unix seconds.
	fn modified(archives: &[DatArchive], place: &Place, name: &str) -> Option<i64> {
		match place {
			Place::Dir(file) => mounts::modified(file),
			Place::Archive(i) => archives[*i].modified(name),
		}
	}

	/// The file the script `path` (without its extension) loads from, and where: its `.nut`
	/// or its `.cnut`, whichever the original loads ([`source_is_newer`]).
	fn script_file(&self, archives: &[DatArchive], path: &str) -> Option<(String, Place)> {
		let (nut, cnut) = (format!("{path}.nut"), format!("{path}.cnut"));
		match (self.locate(archives, &nut), self.locate(archives, &cnut)) {
			(Some(n), Some(c)) => {
				if source_is_newer(Self::modified(archives, &n, &nut), Self::modified(archives, &c, &cnut)) {
					Some((nut, n))
				} else {
					Some((cnut, c))
				}
			}
			(Some(n), None) => Some((nut, n)),
			(None, c) => Some((cnut, c?)),
		}
	}

	/// The script `path` (without its extension), as [`Self::script_file`] picks it, with a
	/// compile `context` for the cache.
	fn load_script(&self, path: &str, context: Option<&dyn Fn() -> u64>) -> Option<LoadedScript> {
		let mut archives = self.archives.borrow_mut();
		let (name, place) = self.script_file(&archives, path)?;
		match (name.ends_with(".nut"), place) {
			(true, Place::Dir(file)) => std::fs::read(file).ok().map(LoadedScript::Source),
			(true, Place::Archive(i)) => self.load_nut(&mut archives[i], &name, context),
			(false, Place::Dir(file)) => transcode_cnut(&std::fs::read(file).ok()?).ok().map(LoadedScript::Bytecode),
			(false, Place::Archive(i)) => self.load_cnut(&mut archives[i], &name),
		}
	}

	/// The `.nut` source `nut` from `archive`. With a cache and a compile `context`,
	/// bytecode an identical earlier compile left there instead.
	fn load_nut(&self, archive: &mut DatArchive, nut: &str, context: Option<&dyn Fn() -> u64>) -> Option<LoadedScript> {
		if let (Some(cache), Some(context)) = (&self.cache, context)
			&& let Some((crc32, size)) = archive.entry_meta(nut)
		{
			let archive_path = archive.path().to_path_buf();
			let key = CacheKey {
				kind: "nut",
				archive: &archive_path,
				entry: nut,
				crc32,
				size,
				context: context(),
			};
			if let Some(bytecode) = cache.get(&key) {
				return Some(LoadedScript::Bytecode(bytecode));
			}
		}
		archive.read(nut).map(LoadedScript::Source)
	}

	/// The compiled `cnut` from `archive`, transcoded to our format (through the cache,
	/// if any).
	fn load_cnut(&self, archive: &mut DatArchive, cnut: &str) -> Option<LoadedScript> {
		let Some(cache) = &self.cache else {
			return transcode_cnut(&archive.read(cnut)?).ok().map(LoadedScript::Bytecode);
		};
		let (crc32, size) = archive.entry_meta(cnut)?;
		let archive_path = archive.path().to_path_buf();
		let key = CacheKey {
			kind: "cnut",
			archive: &archive_path,
			entry: cnut,
			crc32,
			size,
			context: 0,
		};
		if let Some(bytecode) = cache.get(&key) {
			return Some(LoadedScript::Bytecode(bytecode));
		}
		let bytecode = transcode_cnut(&archive.read(cnut)?).ok()?;
		cache.put(&key, &bytecode);
		Some(LoadedScript::Bytecode(bytecode))
	}
}

impl ScriptSource for DatScriptSource {
	fn load(&self, path: &str) -> Option<LoadedScript> {
		self.load_script(path, None)
	}

	fn caches_compiles(&self) -> bool {
		self.cache.is_some()
	}

	fn load_for(&self, path: &str, context: &dyn Fn() -> u64) -> Option<LoadedScript> {
		self.load_script(path, Some(context))
	}

	/// Keep the bytecode compiled from `path`'s `.nut`, when that comes from an archive
	/// (a loose file isn't cached).
	fn compiled(&self, path: &str, context: u64, bytecode: &[u8]) {
		let Some(cache) = &self.cache else {
			return;
		};
		let nut = format!("{path}.nut");
		let mut archives = self.archives.borrow_mut();
		let Some(Place::Archive(i)) = self.locate(&archives, &nut) else {
			return;
		};
		let archive = &mut archives[i];
		let Some((crc32, size)) = archive.entry_meta(&nut) else {
			return;
		};
		let archive_path = archive.path().to_path_buf();
		let key = CacheKey {
			kind: "nut",
			archive: &archive_path,
			entry: &nut,
			crc32,
			size,
			context,
		};
		cache.put(&key, bytecode);
	}

	fn read_file(&self, path: &str) -> Option<Vec<u8>> {
		let mut archives = self.archives.borrow_mut();
		match self.locate(&archives, path)? {
			Place::Dir(file) => std::fs::read(file).ok(),
			Place::Archive(i) => archives[i].read(path),
		}
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

#[cfg(test)]
mod tests;

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
