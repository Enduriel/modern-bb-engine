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
use crate::class_name_hash;
use crate::native::save::original::game_entity::GameEntity;
use crate::native::save::original::record::{Body, Head, SceneEntity, WorldEntityPart, type_id};
use crate::native::save::original::sprite::{
	ALWAYS, BRUSH_BITS, BrushLook, FLIPPED, IGNORE_AMBIENT_COLOR, Sprite, SpriteEntry, VISIBLE,
};
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
	s.brush = sprites.name(&brush_name(sprites, saved.brush));
	s.visible = saved.flags & VISIBLE != 0;
	s.flipped = saved.flags & FLIPPED != 0;
	s.ignore_ambient = saved.flags & IGNORE_AMBIENT_COLOR != 0;
	s.color = SpriteColor::new(argb(saved.color));
	s.scale = saved.scale.into();
	s.rotation = saved.rotation;
	s.offset = (entry.offset.0.into(), entry.offset.1.into());
	s.brush_bits = saved.flags & BRUSH_BITS;
}

/// An entity's sprite as the original saves it.
fn sprite_entry(name: &str, s: &SpriteData, sprites: &Sprites) -> SpriteEntry {
	let look = sprites.with(|r| BrushLook::of(r, &s.brush));
	let mut flags = ALWAYS | s.brush_bits;
	for (on, bit) in [(s.visible, VISIBLE), (s.flipped, FLIPPED), (s.ignore_ambient, IGNORE_AMBIENT_COLOR)] {
		if on {
			flags |= bit;
		}
	}
	SpriteEntry {
		name: name.as_bytes().to_vec(),
		sprite: Sprite {
			brush: look.hash,
			flags,
			color: rgba(s.color.color),
			scale: s.scale as f32,
			rotation: s.rotation,
			saturation: 1.0,
			f64_: 0.1,
			i58: 1,
			v4c: look.v4c,
		},
		stored_color: 0xffff_ffff,
		stored_highlight: 0,
		stored_highlight_factor: 0.0,
		stored_saturation: 1.0,
		offset: (s.offset.0 as f32, s.offset.1 as f32),
	}
}

impl WorldEntities {
	/// Forget every entity (the original's `World.clear` before a load; nothing runs).
	pub(crate) fn clear_for_load(&self) {
		let mut m = self.machine.borrow_mut();
		m.entities.clear();
		m.by_identity.clear();
		m.by_id.clear();
		m.dead.clear();
		m.thinkers = super::think::Thinkers::default();
		m.footprints = super::footprints::Footprints::default();
		crate::native::world_scene_changes::note();
	}

	/// `World.clearScene()`'s share: every entity, the thinker list and the footprints
	/// go, and the player's vision radius is 1 again. The id counter carries on (every
	/// engine object draws from it).
	pub(crate) fn clear_scene(&self) {
		self.clear_for_load();
		self.machine.borrow_mut().player_view.vision_radius = 1.0;
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
		// Its look is the save's now; the turn forgets it if nothing moves.
		let m = self.machine.borrow();
		m.moving.note(m.owner(idx));
		crate::native::world_scene_changes::note();
		Ok(())
	}

	/// A saved footprint: where (`head`), its sprite, when its stay ends, its kind. The
	/// record's `+0x14` holds its visibility (an `f32`).
	pub(crate) fn load_original_footprint(&self, head: &Head, sprite: &Sprite, expires: f32, kind: u16) {
		let mut m = self.machine.borrow_mut();
		let brush = brush_name(&m.sprites, sprite.brush);
		m.footprints.push(Footprint {
			pos: head.pos,
			brush: brush.into(),
			scale: sprite.scale,
			expires,
			visibility: f32::from_bits(head.i14 as u32),
			kind: kind.into(),
			id: head.id.into(),
		});
		if m.ids.peek() <= i64::from(head.id) {
			m.ids.restore(i64::from(head.id) + 1);
		}
		crate::native::world_scene_changes::note();
	}

	/// Every living entity as the original saves it in the scene: those drawn on top
	/// (`RenderedTop`, the `entity` layer) and the others (the `detail` layer), by id.
	pub(crate) fn original_entities(&self, vm: &SQVM) -> Result<(Vec<SceneEntity>, Vec<SceneEntity>), String> {
		let m = self.machine.borrow();
		let sprite_class = m.sprite_class.clone().ok_or("save: sprite class missing")?;
		let (mut top, mut rest) = (Vec::new(), Vec::new());
		let mut order: Vec<&super::EntityRecord> = m.entities.iter().filter(|e| e.alive && e.dies_at.is_none()).collect();
		order.sort_by_key(|e| e.id);
		for rec in order {
			let mut sprites = Vec::new();
			for (name, handle) in rec.sprites.iter() {
				let entry = sprite_class
					.with_payload(vm, handle, |s| sprite_entry(name, s, &m.sprites))
					.ok_or("save: not a sprite")?;
				sprites.push(entry);
			}
			let faction = rec.faction as u8;
			let e = SceneEntity {
				type_: type_id("WorldEntity"),
				head: Head {
					id: rec.id as u32,
					pos: (rec.pos.0 as f32, rec.pos.1 as f32),
					z: rec.pos.1 as f32 as i32 - 20,
					i14: 0,
					rotation: 0.0,
					flags: 0x0070_247d,
					u34: 0,
					children: Vec::new(),
				},
				body: Body::World(WorldEntityPart {
					discovered: rec.discovered,
					ignore_collision: rec.ignore_collision,
					important: rec.important,
					script_hash: class_name_hash(&rec.path) as i32,
					game: GameEntity {
						storing_color: false,
						initialized: true,
						render_callback: false,
						sprite_occlusion: false,
						faction,
						// `setFactionEx`'s `1 << faction` (the shift taken mod 32); never set
						// for faction 0.
						faction_bit: if faction == 0 { 0 } else { 1u32 << (faction % 32) },
						sprites,
					},
				}),
			};
			if rec.rendered_top { top.push(e) } else { rest.push(e) }
		}
		Ok((top, rest))
	}

	/// The footprints as the original saves them (`WorldFootprint`s, in the `detail`
	/// layer).
	pub(crate) fn original_footprints(&self) -> Vec<SceneEntity> {
		let m = self.machine.borrow();
		m.footprints
			.all()
			.iter()
			.map(|f| {
				let look = m.sprites.with(|r| BrushLook::of(r, &f.brush));
				SceneEntity {
					type_: type_id("WorldFootprint"),
					head: Head {
						id: f.id as u32,
						pos: f.pos,
						z: 0,
						// The original keeps its visibility here.
						i14: f.visibility.to_bits() as i32,
						rotation: 0.0,
						flags: 0x0050_2c7d,
						u34: 256,
						children: Vec::new(),
					},
					body: Body::Footprint {
						sprite: Sprite {
							brush: look.hash,
							flags: ALWAYS | VISIBLE | m.sprites.brush_bits(&f.brush, 0),
							color: [0xff; 4],
							scale: f.scale,
							rotation: 0.0,
							saturation: 1.0,
							f64_: 0.0,
							i58: 0,
							v4c: look.v4c,
						},
						time: f.expires,
						kind: f.kind as u16,
					},
				}
			})
			.collect()
	}

	/// Every living entity's id and script, by id descending (the order the original
	/// saves their scripts' data in).
	pub(crate) fn scripts_by_id_descending(&self) -> Vec<(u32, SqStrongRef)> {
		let m = self.machine.borrow();
		let mut v: Vec<(u32, SqStrongRef)> = m
			.entities
			.iter()
			.filter(|e| e.alive && e.dies_at.is_none())
			.filter_map(|e| Some((e.id as u32, e.entity.clone()?)))
			.collect();
		v.sort_by(|a, b| b.0.cmp(&a.0));
		v
	}
}
