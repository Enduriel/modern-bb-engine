//! The archives opened in this process, each parsed once: opening one again (the engine,
//! the renderer, the sound and the page each open the install's archives) shares its
//! [`Index`]. A file changed on disk since (its size or modification time) is
//! opened anew.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use super::central_dir;
use super::index::Index;
use super::shared_file::SharedFile;

struct Opened {
	len: u64,
	modified: Option<SystemTime>,
	index: Arc<Index>,
	file: SharedFile,
}

static OPENED: OnceLock<Mutex<HashMap<PathBuf, Opened>>> = OnceLock::new();

/// The archive at `path`: its index, parsed once per process, and its file.
pub(super) fn open(path: &Path) -> io::Result<(Arc<Index>, SharedFile)> {
	let meta = std::fs::metadata(path)?;
	let (len, modified) = (meta.len(), meta.modified().ok());
	let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
	let mut opened = OPENED.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
	if let Some(o) = opened.get(&key).filter(|o| o.len == len && o.modified == modified) {
		return Ok((o.index.clone(), o.file.clone()));
	}
	let file = SharedFile::new(File::open(path)?)?;
	let index = Arc::new(central_dir::read(&mut file.clone())?);
	opened.insert(
		key,
		Opened {
			len,
			modified,
			index: index.clone(),
			file: file.clone(),
		},
	);
	Ok((index, file))
}

#[cfg(test)]
mod tests {
	use std::io::Write;

	use crate::vfs::DatArchive;

	fn write_zip(path: &std::path::Path, entries: &[(&str, &[u8])]) {
		let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
		for (name, bytes) in entries {
			zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
			zip.write_all(bytes).unwrap();
		}
		zip.finish().unwrap();
	}

	#[test]
	fn openers_share_an_archive_and_a_changed_file_is_opened_anew() {
		let path = std::env::temp_dir().join(format!("bb-opened-{}.zip", std::process::id()));
		write_zip(&path, &[("a.txt", b"first"), ("b.txt", b"second")]);
		let mut one = DatArchive::open(&path).unwrap();
		let mut two = DatArchive::open(&path).unwrap();
		assert_eq!(one.read("b.txt").as_deref(), Some(&b"second"[..]));
		assert_eq!(two.read("a.txt").as_deref(), Some(&b"first"[..]));
		assert_eq!(one.read("a.txt").as_deref(), Some(&b"first"[..]));
		// Rewritten (another size): the next opener sees the new archive.
		write_zip(&path, &[("a.txt", b"rewritten"), ("c.txt", b"third")]);
		let mut three = DatArchive::open(&path).unwrap();
		assert_eq!(three.read("a.txt").as_deref(), Some(&b"rewritten"[..]));
		assert_eq!(three.read("b.txt"), None);
		let _ = std::fs::remove_file(&path);
	}
}
