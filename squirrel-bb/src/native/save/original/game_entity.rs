//! The part of a world entity or unit that `IGameEntity` saves (`0x4a7360` writes,
//! `0x4a70c0` reads): a few flags, the faction, and the sprites.
//!
//! On load the original matches the entries by name to the sprites the script's
//! `onInit` already made; an entry with another name is read and dropped.

use super::super::stream::{SaveReader, SaveWriter};
use super::sprite::SpriteEntry;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GameEntity {
	pub storing_color: bool,
	pub initialized: bool,
	pub render_callback: bool,
	pub sprite_occlusion: bool,
	/// The faction (an int field the original saves as a `u8`).
	pub faction: u8,
	/// `1 << faction`, 32 bits.
	pub faction_bit: u32,
	pub sprites: Vec<SpriteEntry>,
}

impl GameEntity {
	pub fn read(r: &mut SaveReader) -> Result<Self, String> {
		let mut e = GameEntity {
			storing_color: r.bool()?,
			initialized: r.bool()?,
			render_callback: r.bool()?,
			sprite_occlusion: r.bool()?,
			faction: r.u8()?,
			faction_bit: r.u32()?,
			sprites: Vec::new(),
		};
		for _ in 0..r.u8()? {
			e.sprites.push(SpriteEntry::read(r)?);
		}
		Ok(e)
	}

	pub fn write(&self, w: &mut SaveWriter) {
		for b in [self.storing_color, self.initialized, self.render_callback, self.sprite_occlusion] {
			w.bool(b);
		}
		w.u8(self.faction);
		w.u32(self.faction_bit);
		w.u8(self.sprites.len() as u8);
		for s in &self.sprites {
			s.write(w);
		}
	}
}
