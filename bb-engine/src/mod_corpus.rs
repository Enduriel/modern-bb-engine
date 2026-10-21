//! The mod corpus: the installed mods, pinned by name and size in
//! `mod_corpus.txt`. The corpus tests boot, play and compare against goldens with exactly
//! these mods; a different set (a mod updated, added or removed) fails them up front,
//! naming the difference, instead of as some mod's script error halfway through.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::harness;

/// The pinned corpus: `<size> <file name>` per line, `#` comments.
const PINNED: &str = include_str!("mod_corpus.txt");

/// Why the corpus can't be mounted.
#[derive(Debug)]
pub enum CorpusError {
	/// No install with mods here: a test skips (`harness::skip`).
	Absent(String),
	/// The installed mods are not the pinned corpus: a test fails.
	Differs(String),
}

impl fmt::Display for CorpusError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			CorpusError::Absent(why) | CorpusError::Differs(why) => f.write_str(why),
		}
	}
}

/// The installed mods in the data folder `dir` ([`harness::mod_archives`]: those a probe
/// run holds aside count, its own copies don't), in load order, if they are the pinned
/// corpus.
pub fn installed(dir: &Path) -> Result<Vec<PathBuf>, CorpusError> {
	let mods = harness::mod_archives(dir).map_err(|e| CorpusError::Absent(format!("{}: {e}", dir.display())))?;
	if mods.is_empty() {
		return Err(CorpusError::Absent(format!("no mods installed in {}", dir.display())));
	}
	let mut found = BTreeMap::new();
	for path in &mods {
		let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
		let size = std::fs::metadata(harness::locate_mod(path)).map_err(|e| CorpusError::Differs(format!("{name}: {e}")))?.len();
		found.insert(name, size);
	}
	differences(&pinned(), &found).map_err(CorpusError::Differs)?;
	Ok(mods)
}

/// The pinned corpus: file name → size.
fn pinned() -> BTreeMap<String, u64> {
	PINNED
		.lines()
		.filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
		.map(|l| {
			let (size, name) = l.split_once(' ').expect("mod_corpus.txt: <size> <name>");
			(name.to_string(), size.parse().expect("mod_corpus.txt: a size"))
		})
		.collect()
}

/// `Err` naming how `found` differs from `want` (missing, unexpected, other size).
fn differences(want: &BTreeMap<String, u64>, found: &BTreeMap<String, u64>) -> Result<(), String> {
	let mut lines = Vec::new();
	for (name, size) in want {
		match found.get(name) {
			None => lines.push(format!("  missing: {name}")),
			Some(s) if s != size => lines.push(format!("  changed: {name} ({s} bytes, pinned {size})")),
			Some(_) => {}
		}
	}
	lines.extend(found.keys().filter(|n| !want.contains_key(*n)).map(|n| format!("  not pinned: {n}")));
	if lines.is_empty() {
		return Ok(());
	}
	Err(format!(
		"the installed mods are not the pinned corpus (bb-engine/src/mod_corpus.txt):\n{}\n\
		 If the install changed on purpose, update the list (and retake the modded goldens).",
		lines.join("\n")
	))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_pinned_list_reads() {
		let pinned = pinned();
		assert!(pinned.keys().any(|n| n.starts_with("mod_modern_hooks")), "{pinned:?}");
	}

	#[test]
	fn a_different_install_is_named() {
		let map = |items: &[(&str, u64)]| items.iter().map(|(n, s)| (n.to_string(), *s)).collect::<BTreeMap<_, _>>();
		let want = map(&[("a.zip", 1), ("b.zip", 2), ("c.zip", 3)]);
		assert!(differences(&want, &want).is_ok());
		let error = differences(&want, &map(&[("a.zip", 1), ("b.zip", 5), ("legends.zip", 9)])).unwrap_err();
		assert!(error.contains("missing: c.zip"), "{error}");
		assert!(error.contains("changed: b.zip (5 bytes, pinned 2)"), "{error}");
		assert!(error.contains("not pinned: legends.zip"), "{error}");
	}
}
