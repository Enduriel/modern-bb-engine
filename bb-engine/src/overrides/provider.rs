//! What provides files: an archive, or a mounted folder's loose files.

use std::collections::HashSet;
use std::path::Path;

use crate::mounts::{self, Mounts};
use crate::vfs::DatArchive;

pub(super) enum Source<'a> {
	Archive(&'a DatArchive),
	/// A folder, and its files (listed once: a lookup per path would be a file-system
	/// call each).
	Dir(&'a Path, HashSet<String>),
}

pub(super) struct Provider<'a> {
	/// How the log names it: the archive's file name, or which folder's loose files.
	pub(super) label: String,
	/// One of the game's own archives (`data_*.dat`).
	pub(super) vanilla: bool,
	pub(super) source: Source<'a>,
}

/// The providers of `archives` (in load order) and `mounts`, in search order: the data
/// folder, then the archives from the last.
pub(super) fn in_search_order<'a>(archives: &'a [DatArchive], mounts: &'a Mounts) -> Vec<Provider<'a>> {
	let dirs = mounts.dirs().iter().map(|dir| Provider {
		label: format!("loose files in {}/", dir.file_name().map_or(dir.to_string_lossy(), |n| n.to_string_lossy())),
		vanilla: false,
		source: Source::Dir(dir, mounts::files_of(dir).into_iter().collect()),
	});
	let archive = archives.iter().rev().map(|archive| {
		let label = archive.path().file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
		Provider {
			vanilla: label.starts_with("data_") && label.ends_with(".dat"),
			label,
			source: Source::Archive(archive),
		}
	});
	dirs.chain(archive).collect()
}

impl Provider<'_> {
	/// Every file it has.
	pub(super) fn names(&self) -> Vec<String> {
		match &self.source {
			Source::Archive(archive) => archive.names().into_iter().filter(|n| !n.ends_with('/')).collect(),
			Source::Dir(_, files) => files.iter().cloned().collect(),
		}
	}

	pub(super) fn has(&self, name: &str) -> bool {
		match &self.source {
			Source::Archive(archive) => archive.contains(name),
			Source::Dir(_, files) => files.contains(name),
		}
	}

	/// When its file `name` was last modified, as Unix seconds.
	pub(super) fn modified(&self, name: &str) -> Option<i64> {
		match &self.source {
			Source::Archive(archive) => archive.modified(name),
			Source::Dir(dir, _) => mounts::modified(&dir.join(name)),
		}
	}

	/// Whether its file `name` and `other`'s file `other_name` have the same content: by
	/// CRC32 and size between two archives, else by their bytes.
	pub(super) fn same_content(&self, name: &str, other: &Provider, other_name: &str) -> bool {
		if let (Source::Archive(a), Source::Archive(b)) = (&self.source, &other.source) {
			return a.entry_meta(name).is_some() && a.entry_meta(name) == b.entry_meta(other_name);
		}
		let (Some(mine), Some(theirs)) = (self.read(name), other.read(other_name)) else {
			return false;
		};
		mine == theirs
	}

	fn read(&self, name: &str) -> Option<Vec<u8>> {
		match &self.source {
			// The archive is shared: read through a second handle (its index is shared).
			Source::Archive(archive) => DatArchive::open(archive.path()).ok()?.read(name),
			Source::Dir(dir, _) => std::fs::read(dir.join(name)).ok(),
		}
	}
}
