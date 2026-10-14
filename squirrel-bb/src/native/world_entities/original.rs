//! The world entities' share of the original's save layout (see `save::original`): each
//! entity's native part in the scene, the footprints, and the order their scripts'
//! data is saved in.
//!
//! The original re-creates an entity as its record reads: position and flags, then its
//! `Discovered`/`IgnoreCollision`/`Important`, then the script (`create`, `onInit`,
//! `onAfterInit`), then the faction and the saved sprites, matched by name to the ones
//! `onInit` added (`0x4e7dc0`, `0x4a70c0`). A sprite the script didn't add is dropped.

use squirrel::{SQVM, SqStrongRef};

use super::footprints::Footprint;
use super::{SpriteData, WorldEntities, create_entity, ignore_missing};
use crate::native::save::original::record::{Head, WorldEntityPart};
use crate::native::save::original::sprite::{FLIPPED, IGNORE_AMBIENT_COLOR, Sprite, SpriteEntry, VISIBLE};
use crate::native::sprite_color::SpriteColor;
use crate::native::world_grid::brush_name;
use crate::native::{Sprites, world_layout};

/// ARGB from the original's colour bytes (R, G, B, A).
pub(crate) fn argb(rgba: [u8; 4]) -> u32 {
	let [r, g, b, a] = rgba;
	u32::from_be_bytes([a, r, g, b])
}

/// The original's colour bytes (R, G, B, A) from ARGB.
pub(crate) fn rgba(argb: u32) -> [u8; 4] {
	let [a, r, g, b] = argb.to_be_bytes();
	[r, g, b, a]
}

/// Put a saved sprite's state on `s`. Its offset is the entry's (the sprite's own isn't
/// saved).
fn apply_sprite(s: &mut SpriteData, entry: &SpriteEntry, sprites: &Sprites) {
	let saved: &Sprite = &entry.sprite;
	s.brush = brush_name(sprites, saved.brush);
	s.visible = saved.flags & VISIBLE != 0;
	s.flipped = saved.flags & FLIPPED != 0;
	s.ignore_ambient = saved.flags & IGNORE_AMBIENT_COLOR != 0;
	s.color = SpriteColor::new(argb(saved.color));
	s.scale = saved.scale.into();
	s.rotation = saved.rotation;
	s.offset = (entry.offset.0.into(), entry.offset.1.into());
}

impl WorldEntities {
	/// Forget every entity (the original's `World.clear` before a load; nothing runs).
	pub(crate) fn clear_for_load(&self) {
		let mut m = self.machine.borrow_mut();
		m.entities.clear();
		m.by_identity.clear();
		m.by_id.clear();
		m.thinkers = super::think::Thinkers::default();
		m.footprints = super::footprints::Footprints::default();
	}

	/// Re-create the entity of script `path` saved as `head` + `part`, on a map
	/// `dims` (width, height) in size. `top`: it was in the layer drawn on top.
	pub(crate) fn load_original(
		&self,
		vm: &SQVM,
		path: &str,
		top: bool,
		head: &Head,
		part: &WorldEntityPart,
		dims: (i64, i64),
	) -> Result<(), String> {
		let id = i64::from(head.id);
		let pos = (f64::from(head.pos.0), f64::from(head.pos.1));
		let (q, r) = world_layout::world_to_tile(pos.0, pos.1, dims.0, dims.1);
		let (entity, idx) = create_entity(&self.machine, vm, path, id, q, r)?;
		{
			let mut m = self.machine.borrow_mut();
			if m.ids.peek() <= id {
				m.ids.restore(id + 1);
			}
			let rec = &mut m.entities[idx];
			rec.pos = pos;
			rec.rendered_top = top;
			rec.discovered = part.discovered;
			rec.ignore_collision = part.ignore_collision;
			rec.important = part.important;
		}
		entity.call_method(vm, "onInit", ())?;
		ignore_missing(entity.call_method(vm, "onAfterInit", ()))?;
		let (sprite_class, sprites) = {
			let mut m = self.machine.borrow_mut();
			m.entities[idx].faction = part.game.faction.into();
			(m.sprite_class.clone().ok_or("load: sprite class missing")?, m.sprites.clone())
		};
		for entry in &part.game.sprites {
			let name = String::from_utf8_lossy(&entry.name).into_owned();
			let handle = self.machine.borrow().entities[idx].sprites.get(&name).cloned();
			if let Some(handle) = handle {
				sprite_class.with_payload(vm, &handle, |s| apply_sprite(s, entry, &sprites));
			}
		}
		Ok(())
	}

	/// A saved footprint: where (`head`), its sprite, when its stay ends, its kind. The
	/// record's `+0x14` holds its visibility (an `f32`).
	pub(crate) fn load_original_footprint(&self, head: &Head, sprite: &Sprite, expires: f32, kind: u16) {
		let mut m = self.machine.borrow_mut();
		let brush = brush_name(&m.sprites, sprite.brush);
		m.footprints.push(Footprint {
			pos: head.pos,
			brush,
			scale: sprite.scale,
			expires,
			visibility: f32::from_bits(head.i14 as u32),
			kind: kind.into(),
		});
	}

	/// Every living entity's id and script, by id descending (the order the original
	/// saves their scripts' data in).
	pub(crate) fn scripts_by_id_descending(&self) -> Vec<(u32, SqStrongRef)> {
		let m = self.machine.borrow();
		let mut v: Vec<(u32, SqStrongRef)> = m
			.entities
			.iter()
			.filter(|e| e.alive && e.dies_at.is_none())
			.map(|e| (e.id as u32, e.entity.clone()))
			.collect();
		v.sort_by(|a, b| b.0.cmp(&a.0));
		v
	}
}
