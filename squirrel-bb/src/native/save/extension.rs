//! This engine's extension of the original's campaign save: what
//! the engine keeps beyond the original's state rides after the fog of war, the
//! original's last block. The original lists and loads such a save as if the extension
//! weren't there (probed 2026-10-19), and drops it when it saves the campaign again, so
//! every section's owner copes with its section missing.
//!
//! Layout, little-endian:
//!
//! * `BBME`, `u32` extension version ([`VERSION`]), `u32` section count;
//! * the index: per section its name (`u16` length + bytes), `u64` offset from the
//!   extension's start, `u64` length;
//! * the sections' bytes;
//! * the trailer: `u64` length of everything above, then `BBME` again. A reader finds
//!   the extension from the end of the file, without walking the body.
//!
//! Sections are named, so a section whose owner isn't there (a native mod that isn't
//! loaded, a newer engine's) is skipped on load and written back unchanged on the next
//! save.

use std::collections::BTreeMap;

const MAGIC: &[u8; 4] = b"BBME";
/// The extension's layout version.
pub(crate) const VERSION: u32 = 1;
/// The trailer: the extension's length and the magic.
const TRAILER: usize = 8 + 4;

/// The extension holding `sections` (name → bytes, written in name order).
pub(crate) fn encode(sections: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
	let index_len: usize = sections.keys().map(|k| 2 + k.len() + 8 + 8).sum();
	let mut out = Vec::new();
	out.extend_from_slice(MAGIC);
	out.extend_from_slice(&VERSION.to_le_bytes());
	out.extend_from_slice(&(sections.len() as u32).to_le_bytes());
	let mut offset = (MAGIC.len() + 4 + 4 + index_len) as u64;
	for (name, bytes) in sections {
		out.extend_from_slice(&(name.len() as u16).to_le_bytes());
		out.extend_from_slice(name.as_bytes());
		out.extend_from_slice(&offset.to_le_bytes());
		out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
		offset += bytes.len() as u64;
	}
	for bytes in sections.values() {
		out.extend_from_slice(bytes);
	}
	let len = out.len() as u64;
	out.extend_from_slice(&len.to_le_bytes());
	out.extend_from_slice(MAGIC);
	out
}

/// Takes `n` bytes from `bytes` at `*at`.
fn take<'a>(bytes: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8], String> {
	let end = at.checked_add(n).filter(|&e| e <= bytes.len()).ok_or("the save's extension is cut short")?;
	let s = &bytes[*at..end];
	*at = end;
	Ok(s)
}

fn u64_at(bytes: &[u8], at: &mut usize) -> Result<u64, String> {
	Ok(u64::from_le_bytes(take(bytes, at, 8)?.try_into().unwrap()))
}

/// The sections of the extension that `rest` (all of the file after the fog) ends
/// with; none if `rest` is empty. An extension of a later layout version is an error.
pub(crate) fn decode(rest: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
	if rest.is_empty() {
		return Ok(BTreeMap::new());
	}
	if rest.len() < TRAILER || &rest[rest.len() - 4..] != MAGIC {
		return Err("the save has bytes after the fog of war that aren't this engine's extension".into());
	}
	let mut at = rest.len() - TRAILER;
	let len = u64_at(rest, &mut at)? as usize;
	let start = (rest.len() - TRAILER).checked_sub(len).ok_or("the save's extension is cut short")?;
	let ext = &rest[start..rest.len() - TRAILER];
	let mut at = 0;
	if take(ext, &mut at, 4)? != MAGIC {
		return Err("the save's extension doesn't start where its trailer says".into());
	}
	let version = u32::from_le_bytes(take(ext, &mut at, 4)?.try_into().unwrap());
	if version > VERSION {
		return Err(format!("the save's extension is of version {version}; this engine reads up to {VERSION}"));
	}
	let count = u32::from_le_bytes(take(ext, &mut at, 4)?.try_into().unwrap());
	let mut sections = BTreeMap::new();
	for _ in 0..count {
		let n = u16::from_le_bytes(take(ext, &mut at, 2)?.try_into().unwrap()) as usize;
		let name = String::from_utf8_lossy(take(ext, &mut at, n)?).into_owned();
		let mut o = u64_at(ext, &mut at)? as usize;
		let l = u64_at(ext, &mut at)? as usize;
		sections.insert(name, take(ext, &mut o, l)?.to_vec());
	}
	Ok(sections)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sections_round_trip_and_are_found_from_the_end() {
		let mut sections = BTreeMap::new();
		sections.insert("b.mod".to_string(), vec![1, 2, 3]);
		sections.insert("a".to_string(), vec![]);
		sections.insert("engine.factions".to_string(), vec![9; 300]);
		let ext = encode(&sections);
		assert_eq!(&ext[..4], b"BBME");
		assert_eq!(&ext[ext.len() - 4..], b"BBME");
		assert_eq!(decode(&ext).unwrap(), sections);
	}

	#[test]
	fn no_extension_is_no_sections() {
		assert!(decode(&[]).unwrap().is_empty());
		assert!(decode(&encode(&BTreeMap::new())).unwrap().is_empty());
	}

	#[test]
	fn foreign_or_damaged_tails_are_errors() {
		assert!(decode(&[0; 1000]).is_err());
		let mut sections = BTreeMap::new();
		sections.insert("x".to_string(), vec![7; 10]);
		let ext = encode(&sections);
		assert!(decode(&ext[5..]).is_err());
		let mut later = ext.clone();
		later[4] = 2;
		assert!(decode(&later).unwrap_err().contains("version 2"));
	}
}
