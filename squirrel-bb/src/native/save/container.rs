//! The original's file container, around every `.sav` it writes: campaign saves and the
//! `PersistenceManager` storages mods keep (MSU's `MSU#…` files). Disassembled
//! (`0x5636a0` writes, `0x563260` reads):
//!
//! `u16` 0xbb, `u8` 2, `u32` version, `u64` creation time, `u64` modification time (Unix
//! seconds), the label, then the metadata: a `u16` count of string key/value pairs in
//! key order, and `u16` 0xbb. The body follows. Strings are a `u16` length and bytes.

use std::collections::BTreeMap;

use super::meta::Meta;
use super::stream::{SaveReader, SaveWriter, StrLen};

const MAGIC: u16 = 0xbb;
const FORMAT: u8 = 2;

/// Whether `bytes` start like a file in the original's container.
pub(crate) fn is_original(bytes: &[u8]) -> bool {
	bytes.starts_with(&[0xbb, 0, FORMAT])
}

/// The bytes of a label or metadata string as stored. The original's are bytes; ours
/// UTF-8 (anything else read back is kept lossily).
fn text(bytes: Vec<u8>) -> String {
	String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// A file: the header with `meta`, then `body`.
pub(crate) fn encode(meta: &Meta, body: &[u8]) -> Vec<u8> {
	let mut w = SaveWriter {
		strings: StrLen::U16,
		..SaveWriter::default()
	};
	w.u16(MAGIC);
	w.u8(FORMAT);
	w.u32(meta.version as u32);
	w.i64(meta.created);
	w.i64(meta.modified);
	w.str(&meta.name);
	w.u16(meta.data.len() as u16);
	for (k, v) in &meta.data {
		w.str(k);
		w.str(v);
	}
	w.u16(MAGIC);
	w.buf.extend_from_slice(body);
	w.buf
}

/// Split file `bytes` (stored as `file_name`) into its metadata and a reader at the
/// body.
pub(crate) fn decode(bytes: Vec<u8>, file_name: &str) -> Result<(Meta, SaveReader), String> {
	if !is_original(&bytes) {
		return Err(format!("{file_name}: not a save file"));
	}
	let mut r = SaveReader::with(bytes, StrLen::U16);
	r.seek(3);
	let version = r.u32()?;
	let created = r.i64()?;
	let modified = r.i64()?;
	let name = text(r.bytes()?);
	let mut data = BTreeMap::new();
	for _ in 0..r.u16()? {
		let k = text(r.bytes()?);
		data.insert(k, text(r.bytes()?));
	}
	if r.u16()? != MAGIC {
		return Err(format!("{file_name}: the save's metadata doesn't end where it should"));
	}
	let meta = Meta {
		version: version.into(),
		name,
		file_name: file_name.to_string(),
		created,
		modified,
		data,
	};
	Ok((meta, r))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The start of a save the original wrote (vanilla, day 1), through its metadata.
	const ORIGINAL: &str = "bb0002410000003aa0ce6a000000003aa0ce6a0000000008006175746f736176\
		650800060062616e6e6572090062616e6e65725f30310400646179730100310a\
		00646966666963756c74790100310b00646966666963756c7479320100310300\
		646c630300333433090067726f75706e616d650d0050726f626520436f6d7061\
		6e79070069726f6e6d616e01003008006d6f646966696564010031bb00";

	fn hex(s: &str) -> Vec<u8> {
		(0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
	}

	#[test]
	fn reads_and_rewrites_the_originals_header() {
		let mut bytes = hex(ORIGINAL);
		bytes.extend_from_slice(&[0xbb, 0, 1, 2]);
		let (meta, mut body) = decode(bytes.clone(), "autosave").unwrap();
		assert_eq!(meta.version, 65);
		assert_eq!(meta.name, "autosave");
		assert_eq!(meta.created, 1_791_926_330);
		assert_eq!(meta.get_string("groupname"), "Probe Company");
		assert_eq!(meta.get_int("dlc"), 343);
		assert_eq!(body.u16().unwrap(), 0xbb);
		assert_eq!(encode(&meta, &[0xbb, 0, 1, 2]), bytes);
	}

	#[test]
	fn rejects_other_files() {
		assert!(decode(b"BBMESAVE".to_vec(), "x").is_err());
		assert!(!is_original(b"BBMESAVE"));
	}
}
