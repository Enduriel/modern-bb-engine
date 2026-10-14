//! The fog of war as the original saves it, last in a campaign save (`0x4a29b0`
//! writes, `0x4a2800` reads): the 2048×2048 mask, one byte per pixel (its error text
//! calls it "FoW").
//!
//! Written: `u8` 0xbb, `u32` length, the bytes, `bool` false (not compressed). Read: the
//! 0xbb from version 36 on; from version 35 the data may be compressed (always before
//! version 45, else when the bool says so).

use super::super::stream::{SaveReader, SaveWriter};

const MARK: u8 = 0xbb;

/// The mask's bytes, from a save of `version`.
pub(crate) fn read(r: &mut SaveReader, version: i64) -> Result<Vec<u8>, String> {
	if version >= 36 && r.u8()? != MARK {
		return Err("Corrupt data before FoW".to_string());
	}
	let len = r.u32()? as usize;
	let data = r.raw(len)?.to_vec();
	let compressed = version >= 35 && (version < 45 || r.bool()?);
	if compressed {
		return Err("a compressed fog of war (saves before version 45) isn't supported".to_string());
	}
	Ok(data)
}

pub(crate) fn write(w: &mut SaveWriter, mask: &[u8]) {
	w.u8(MARK);
	w.u32(mask.len() as u32);
	w.buf.extend_from_slice(mask);
	w.bool(false);
}
