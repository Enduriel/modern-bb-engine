//! A sprite as the original saves it (`0x52d860` writes, `0x52d780` reads), and an
//! entity's entry for one of its sprites (in `IGameEntity`, see [`super::game_entity`]).

use super::super::stream::{SaveReader, SaveWriter};
use super::wire;

/// A sprite. Not saved: its own offset (scripts set it again), highlight, torque.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sprite {
	/// The brush's name hash (`bb_assets::name_hash` of its name); 0 for none.
	pub brush: u64,
	/// bit0 Visible, bit10 flipped horizontally, bit23 IgnoreCameraFlip, bit27
	/// IgnoreAmbientColor.
	pub flags: u32,
	/// RGBA.
	pub color: [u8; 4],
	pub scale: f32,
	pub rotation: f32,
	pub saturation: f32,
	/// Unnamed (`+0x64`, 0.1 seen).
	pub f64_: f32,
	/// Unnamed (`+0x58`, 1 seen).
	pub i58: i8,
	/// Unnamed (`+0x4c`), which setting the brush also sets.
	pub v4c: (f32, f32),
}

pub(crate) const VISIBLE: u32 = 1;
pub(crate) const FLIPPED: u32 = 1 << 10;
pub(crate) const IGNORE_CAMERA_FLIP: u32 = 1 << 23;
pub(crate) const IGNORE_AMBIENT_COLOR: u32 = 1 << 27;

impl Sprite {
	pub fn read(r: &mut SaveReader) -> Result<Self, String> {
		Ok(Sprite {
			brush: r.i64()? as u64,
			flags: r.u32()?,
			color: wire::bytes4(r)?,
			scale: r.f32()?,
			rotation: r.f32()?,
			saturation: r.f32()?,
			f64_: r.f32()?,
			i58: r.u8()? as i8,
			v4c: wire::vec2(r)?,
		})
	}

	pub fn write(&self, w: &mut SaveWriter) {
		w.i64(self.brush as i64);
		w.u32(self.flags);
		w.buf.extend_from_slice(&self.color);
		w.f32(self.scale);
		w.f32(self.rotation);
		w.f32(self.saturation);
		w.f32(self.f64_);
		w.u8(self.i58 as u8);
		wire::write_vec2(w, self.v4c);
	}
}

/// An entity's sprite: its name, the sprite, the colours `storeSpriteColors` keeps, and
/// the sprite's offset on the entity.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpriteEntry {
	pub name: Vec<u8>,
	pub sprite: Sprite,
	pub stored_color: u32,
	pub stored_highlight: u32,
	pub stored_highlight_factor: f32,
	pub stored_saturation: f32,
	pub offset: (f32, f32),
}

impl SpriteEntry {
	pub fn read(r: &mut SaveReader) -> Result<Self, String> {
		Ok(SpriteEntry {
			name: r.bytes()?,
			sprite: Sprite::read(r)?,
			stored_color: r.u32()?,
			stored_highlight: r.u32()?,
			stored_highlight_factor: r.f32()?,
			stored_saturation: r.f32()?,
			offset: wire::vec2(r)?,
		})
	}

	pub fn write(&self, w: &mut SaveWriter) {
		w.bytes(&self.name);
		self.sprite.write(w);
		w.u32(self.stored_color);
		w.u32(self.stored_highlight);
		w.f32(self.stored_highlight_factor);
		w.f32(self.stored_saturation);
		wire::write_vec2(w, self.offset);
	}
}

/// The sprite flags that are always set (bit1, bit12).
pub(crate) const ALWAYS: u32 = 0x1002;

/// The flag bits a sprite's brush calls set (`0x4` "not animating", `0x10` the brush's
/// flag bit 3; see `Sprites::brush_bits`).
pub(crate) const BRUSH_BITS: u32 = 0x14;

/// What setting a brush gives a sprite, as the original saves it (`0x52bf50`): the
/// brush's stored hash and its `unknown_offset` pair (the saved `v4c`). No brush:
/// zeros.
pub(crate) struct BrushLook {
	pub hash: u64,
	pub v4c: (f32, f32),
}

impl BrushLook {
	pub fn of(registry: &bb_assets::SpriteRegistry, name: &str) -> Self {
		match registry.get(name) {
			Some((_, b)) => BrushLook {
				hash: b.hash,
				v4c: (b.unknown_offset[0], b.unknown_offset[1]),
			},
			None => BrushLook { hash: 0, v4c: (0.0, 0.0) },
		}
	}
}
