//! Small value encodings the original's records share.

use super::super::stream::{SaveReader, SaveWriter};

/// Two `f32`s (the original writes these as 8 raw bytes).
pub(crate) fn vec2(r: &mut SaveReader) -> Result<(f32, f32), String> {
	Ok((r.f32()?, r.f32()?))
}

pub(crate) fn write_vec2(w: &mut SaveWriter, v: (f32, f32)) {
	w.f32(v.0);
	w.f32(v.1);
}

pub(crate) fn bytes4(r: &mut SaveReader) -> Result<[u8; 4], String> {
	Ok(r.raw(4)?.try_into().expect("4 bytes"))
}

pub(crate) fn i32(r: &mut SaveReader) -> Result<i32, String> {
	Ok(r.u32()? as i32)
}
