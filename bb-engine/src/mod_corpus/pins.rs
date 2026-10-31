//! The pin list (`mod_corpus.txt`): every file of the reference corpus by its path in
//! the corpus folder, with its size and SHA-256, and how a folder's files differ from it.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// A file's size and SHA-256 (lowercase hex).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
	pub size: u64,
	pub sha256: String,
}

/// The pins in a list's `text`: `<sha256> <size> <path>` per line (the path, relative
/// to the corpus folder with `/` between its parts, may hold spaces), `#` comments.
pub fn parse(text: &str) -> Result<BTreeMap<String, Pin>, String> {
	let mut pins = BTreeMap::new();
	for (n, line) in text.lines().enumerate() {
		if line.trim().is_empty() || line.starts_with('#') {
			continue;
		}
		let bad = || format!("line {}: not `<sha256> <size> <path>`: {line}", n + 1);
		let mut parts = line.splitn(3, ' ');
		let (Some(sha256), Some(size), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
			return Err(bad());
		};
		if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
			return Err(bad());
		}
		let size = size.parse().map_err(|_| bad())?;
		pins.insert(path.to_string(), Pin { size, sha256: sha256.to_ascii_lowercase() });
	}
	Ok(pins)
}

/// The files in the folders `subfolders` of `dir`, by their paths in `dir`
/// (`<subfolder>/<name>`), each with its size and SHA-256. A missing subfolder has none.
pub fn found_in(dir: &Path, subfolders: &[&str]) -> io::Result<BTreeMap<String, Pin>> {
	let mut found = BTreeMap::new();
	for sub in subfolders {
		let entries = match std::fs::read_dir(dir.join(sub)) {
			Ok(entries) => entries,
			Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
			Err(e) => return Err(e),
		};
		for entry in entries {
			let path = entry?.path();
			let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
			let size = std::fs::metadata(&path)?.len();
			let sha256 = bb_sha256::file_hex(&path)?;
			found.insert(format!("{sub}/{name}"), Pin { size, sha256 });
		}
	}
	Ok(found)
}

/// Each way `found` differs from `want`, one line each: a file missing, of another
/// size or content, or not pinned. Empty when they are the same.
pub fn differences(want: &BTreeMap<String, Pin>, found: &BTreeMap<String, Pin>) -> Vec<String> {
	let mut lines = Vec::new();
	for (path, pin) in want {
		match found.get(path) {
			None => lines.push(format!("missing: {path}")),
			Some(f) if f.size != pin.size => lines.push(format!("changed: {path} ({} bytes, pinned {})", f.size, pin.size)),
			Some(f) if f.sha256 != pin.sha256 => {
				lines.push(format!("changed: {path} (sha256 {}, pinned {})", f.sha256, pin.sha256))
			}
			Some(_) => {}
		}
	}
	lines.extend(found.keys().filter(|p| !want.contains_key(*p)).map(|p| format!("not pinned: {p}")));
	lines
}

#[cfg(test)]
mod tests {
	use super::*;

	const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
	const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

	fn pin(size: u64, sha256: &str) -> Pin {
		Pin { size, sha256: sha256.to_string() }
	}

	#[test]
	fn a_list_reads_with_spaces_in_names_and_comments() {
		let text = format!("# comment\n\n{A} 12 mods/mod_x 1.0 2026.zip\n{} 3 storages/MSU#mod_msu#ModSettings.sav\n", B.to_uppercase());
		let pins = parse(&text).unwrap();
		assert_eq!(pins.len(), 2);
		assert_eq!(pins["mods/mod_x 1.0 2026.zip"], pin(12, A));
		assert_eq!(pins["storages/MSU#mod_msu#ModSettings.sav"], pin(3, B));
		assert!(parse("12 mods/old_format.zip").unwrap_err().contains("line 1"));
		assert!(parse(&format!("{A} twelve mods/a.zip")).is_err());
	}

	#[test]
	fn each_difference_is_named() {
		let want: BTreeMap<_, _> =
			[("mods/a.zip", pin(1, A)), ("mods/b.zip", pin(2, A)), ("mods/c.zip", pin(3, A)), ("storages/s.sav", pin(4, A))]
				.into_iter()
				.map(|(p, x)| (p.to_string(), x))
				.collect();
		assert!(differences(&want, &want).is_empty());
		let mut found = want.clone();
		found.remove("mods/c.zip");
		found.insert("mods/b.zip".into(), pin(5, A));
		found.insert("storages/s.sav".into(), pin(4, B));
		found.insert("mods/legends.zip".into(), pin(9, A));
		let lines = differences(&want, &found);
		assert_eq!(
			lines,
			[
				"changed: mods/b.zip (5 bytes, pinned 2)".to_string(),
				"missing: mods/c.zip".to_string(),
				format!("changed: storages/s.sav (sha256 {B}, pinned {A})"),
				"not pinned: mods/legends.zip".to_string(),
			]
		);
	}
}
