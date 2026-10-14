//! The world camera as the original saves it (`0x52edc0` writes, `0x52ecc0` reads; the
//! camera object at World+0x14).

use super::super::stream::{SaveReader, SaveWriter};
use super::wire;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Camera {
	/// `+0x8`: the position.
	pub pos: (f32, f32),
	/// `+0x10`.
	pub v10: (f32, f32),
	/// `+0x68`.
	pub v68: (f32, f32),
	/// `+0x58`.
	pub i58: i8,
	/// `+0x73`.
	pub b73: bool,
	/// `+0x4c`: the zoom.
	pub zoom: f32,
}

impl Camera {
	pub fn read(r: &mut SaveReader) -> Result<Self, String> {
		Ok(Camera {
			pos: wire::vec2(r)?,
			v10: wire::vec2(r)?,
			v68: wire::vec2(r)?,
			i58: r.u8()? as i8,
			b73: r.bool()?,
			zoom: r.f32()?,
		})
	}

	pub fn write(&self, w: &mut SaveWriter) {
		wire::write_vec2(w, self.pos);
		wire::write_vec2(w, self.v10);
		wire::write_vec2(w, self.v68);
		w.u8(self.i58 as u8);
		w.bool(self.b73);
		w.f32(self.zoom);
	}
}

/// The world clock (World+0x1c4's `Time`, the seconds into the day's phase, the phase).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Clock {
	pub time: f32,
	pub phase_seconds: f32,
	pub phase: u8,
}

impl Clock {
	pub fn read(r: &mut SaveReader) -> Result<Self, String> {
		Ok(Clock {
			time: r.f32()?,
			phase_seconds: r.f32()?,
			phase: r.u8()?,
		})
	}

	pub fn write(&self, w: &mut SaveWriter) {
		w.f32(self.time);
		w.f32(self.phase_seconds);
		w.u8(self.phase);
	}
}
