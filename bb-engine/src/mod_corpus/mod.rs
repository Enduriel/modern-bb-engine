//! The reference mod corpus the corpus tests mount: a copy of a set of mod archives and
//! of the mod storages they read (MSU's settings, say), kept outside the repository
//! (`location`) and pinned file by file, by size and SHA-256, in `mod_corpus.txt`. The
//! corpus tests boot, play and compare against goldens with exactly these files, never
//! with the mods installed in the game's `data/` or the storages in its `savegames/`,
//! so updating the installed mods changes nothing for them.
//!
//! The folder holds `mods/` (the archives) and `storages/` (the `<mod>#<name>.sav`
//! files). `scripts/refresh_mod_corpus.py` makes it from the installed mods, rewrites
//! the pin list and says which goldens to retake: moving the corpus forward is a
//! deliberate step. A corpus that isn't the pinned one fails the tests, naming each
//! difference; none at all skips them (`harness::skip`: fails unless skips are allowed),
//! saying how to make one.

mod location;
mod pins;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use location::{FOLDER, VAR};

/// The pin list: `<sha256> <size> <path in the corpus folder>` per line, `#` comments.
const PINNED: &str = include_str!("../mod_corpus.txt");
/// The corpus folder's archives.
pub const MODS: &str = "mods";
/// The corpus folder's mod storages.
pub const STORAGES: &str = "storages";

/// How to make the corpus, for the messages.
const HOW: &str = "make it with `scripts/refresh_mod_corpus.py --to DIR` (DIR a folder named mod-corpus beside \
	the checkout, or set BB_MOD_CORPUS=DIR); it copies the installed mods and their storages";

/// Why the corpus can't be mounted.
#[derive(Clone, Debug)]
pub enum CorpusError {
	/// No corpus on this machine: a test skips (`harness::skip`).
	Absent(String),
	/// The corpus is not the pinned one: a test fails.
	Differs(String),
}

impl fmt::Display for CorpusError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			CorpusError::Absent(why) | CorpusError::Differs(why) => f.write_str(why),
		}
	}
}

/// The reference corpus, checked against its pins.
#[derive(Clone, Debug)]
pub struct Corpus {
	dir: PathBuf,
	mods: Vec<PathBuf>,
}

impl Corpus {
	/// The corpus folder.
	pub fn dir(&self) -> &Path {
		&self.dir
	}

	/// The folder of the corpus's archives (for `bb --mods-dir`).
	pub fn mods_dir(&self) -> PathBuf {
		self.dir.join(MODS)
	}

	/// The archives, in the order the original mounts mods (by name, after the
	/// `data_*.dat`s).
	pub fn mods(&self) -> &[PathBuf] {
		&self.mods
	}

	/// The archive whose name starts with `prefix` (the last such, by name).
	pub fn find(&self, prefix: &str) -> Option<&PathBuf> {
		self.mods.iter().rfind(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix)))
	}

	/// Copies the corpus's mod storages into the folder `dest` (a game's savegames folder
	/// for the test: the mods may write there, never into the corpus). Their names.
	pub fn copy_storages(&self, dest: &Path) -> io::Result<Vec<String>> {
		let mut names = Vec::new();
		let entries = match std::fs::read_dir(self.dir.join(STORAGES)) {
			Ok(entries) => entries,
			Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(names),
			Err(e) => return Err(e),
		};
		for entry in entries {
			let path = entry?.path();
			let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
			std::fs::copy(&path, dest.join(&name))?;
			names.push(name);
		}
		names.sort();
		Ok(names)
	}
}

/// The reference corpus (`location`), if it is the pinned one. Checked once per process.
pub fn reference() -> Result<Corpus, CorpusError> {
	static CHECKED: OnceLock<Result<Corpus, CorpusError>> = OnceLock::new();
	CHECKED
		.get_or_init(|| {
			let dir = location::find().map_err(|why| CorpusError::Absent(format!("no reference mod corpus: {why}; {HOW}")))?;
			let pinned = pins::parse(PINNED).map_err(|e| CorpusError::Differs(format!("bb-engine/src/mod_corpus.txt: {e}")))?;
			open(&dir, &pinned)
		})
		.clone()
}

/// The corpus in `dir`, if its files are `pinned`.
fn open(dir: &Path, pinned: &std::collections::BTreeMap<String, pins::Pin>) -> Result<Corpus, CorpusError> {
	let found = pins::found_in(dir, &[MODS, STORAGES]).map_err(|e| CorpusError::Differs(format!("{}: {e}", dir.display())))?;
	if found.is_empty() {
		return Err(CorpusError::Absent(format!("the reference mod corpus {} is empty; {HOW}", dir.display())));
	}
	let lines = pins::differences(pinned, &found);
	if !lines.is_empty() {
		return Err(CorpusError::Differs(format!(
			"the reference mod corpus {} is not the pinned one (bb-engine/src/mod_corpus.txt):\n  {}\n\
			 Moving it forward on purpose is `scripts/refresh_mod_corpus.py`, which rewrites the list; otherwise \
			 restore the folder (or make it again from the same mods).",
			dir.display(),
			lines.join("\n  ")
		)));
	}
	let mods = found.keys().filter_map(|p| p.strip_prefix(&format!("{MODS}/"))).map(|name| dir.join(MODS).join(name)).collect();
	Ok(Corpus { dir: dir.to_path_buf(), mods })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_pinned_list_reads() {
		let pinned = pins::parse(PINNED).unwrap();
		assert!(pinned.keys().any(|p| p.starts_with("mods/mod_modern_hooks")), "{pinned:?}");
		assert!(pinned.keys().all(|p| p.starts_with("mods/") || p.starts_with("storages/")), "{pinned:?}");
	}

	/// A corpus folder of `files` (path, content) for one test.
	fn corpus(test: &str, files: &[(&str, &str)]) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("bb-mod-corpus-{test}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		for (path, content) in files {
			let path = dir.join(path);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(path, content).unwrap();
		}
		dir
	}

	fn pin_list(files: &[(&str, &str)]) -> std::collections::BTreeMap<String, pins::Pin> {
		let text: String = files
			.iter()
			.map(|(path, content)| format!("{} {} {path}\n", bb_steam::sha256::hex(content.as_bytes()), content.len()))
			.collect();
		pins::parse(&text).unwrap()
	}

	const FILES: [(&str, &str); 3] =
		[("mods/mod_b.zip", "b"), ("mods/mod_a 1.0.zip", "a"), ("storages/MSU#mod_msu#ModSettings.sav", "settings")];

	#[test]
	fn the_pinned_corpus_opens_in_load_order_and_its_storages_copy() {
		let dir = corpus("opens", &FILES);
		let corpus = open(&dir, &pin_list(&FILES)).unwrap();
		assert_eq!(corpus.mods(), [dir.join("mods/mod_a 1.0.zip"), dir.join("mods/mod_b.zip")]);
		assert_eq!(corpus.find("mod_b"), Some(&dir.join("mods/mod_b.zip")));
		assert_eq!(corpus.mods_dir(), dir.join("mods"));
		let dest = dir.join("saves");
		std::fs::create_dir_all(&dest).unwrap();
		assert_eq!(corpus.copy_storages(&dest).unwrap(), ["MSU#mod_msu#ModSettings.sav"]);
		assert_eq!(std::fs::read_to_string(dest.join("MSU#mod_msu#ModSettings.sav")).unwrap(), "settings");
		std::fs::remove_dir_all(&dir).unwrap();
	}

	#[test]
	fn an_edited_corpus_fails_naming_the_file_and_an_empty_one_is_absent() {
		let dir = corpus("edited", &FILES);
		// Same size, other content: only the hash tells.
		std::fs::write(dir.join("storages/MSU#mod_msu#ModSettings.sav"), "SETTINGS").unwrap();
		std::fs::write(dir.join("mods/mod_c.zip"), "c").unwrap();
		match open(&dir, &pin_list(&FILES)) {
			Err(CorpusError::Differs(why)) => {
				assert!(why.contains("changed: storages/MSU#mod_msu#ModSettings.sav (sha256 "), "{why}");
				assert!(why.contains("not pinned: mods/mod_c.zip"), "{why}");
			}
			other => panic!("{other:?}"),
		}
		let empty = corpus("empty", &[]);
		std::fs::create_dir_all(&empty).unwrap();
		assert!(matches!(open(&empty, &pin_list(&FILES)), Err(CorpusError::Absent(why)) if why.contains("refresh_mod_corpus.py")));
		std::fs::remove_dir_all(&dir).unwrap();
		std::fs::remove_dir_all(&empty).unwrap();
	}
}
