//! The rosters' share of the original's save layout (see `save::original`): a unit is
//! re-created from its record (`0x4c36f0`): position, then its script (`create`,
//! `onInit`, `onAfterInit`), then its faction and saved sprites, matched by name to
//! the ones `onInit` added. Its script's data follows (the caller runs `onDeserialize`).

use squirrel::{SQVM, SqStrongRef};

use super::{GUEST_ROSTER_ID, PLAYER_ROSTER_ID, TacticalEntities, UnitSpriteData, create_unit, ignore_missing};
use crate::native::Sprites;
use crate::class_name_hash;
use crate::native::save::original::game_entity::GameEntity;
use crate::native::save::original::record::{Head, TacticalNative};
use crate::native::save::original::sprite::{
	ALWAYS, BRUSH_BITS, BrushLook, FLIPPED, IGNORE_AMBIENT_COLOR, IGNORE_CAMERA_FLIP, Sprite, SpriteEntry, VISIBLE,
};
use crate::native::save::original::world::{Roster, Rosters, SavedUnit};
use crate::native::sprite_color::SpriteColor;
use crate::native::world_entities::{argb, rgba};
use crate::native::world_grid::brush_name;

/// Our id of roster `r`.
fn our_roster_id(r: Roster) -> i64 {
	match r {
		Roster::Player => PLAYER_ROSTER_ID,
		Roster::Guests => GUEST_ROSTER_ID,
		Roster::Created(id) => id.into(),
	}
}

fn apply_sprite(s: &mut UnitSpriteData, entry: &SpriteEntry, sprites: &Sprites) {
	let saved = &entry.sprite;
	s.brush = brush_name(sprites, saved.brush);
	s.visible = saved.flags & VISIBLE != 0;
	s.flipped = saved.flags & FLIPPED != 0;
	s.ignore_camera_flip = saved.flags & IGNORE_CAMERA_FLIP != 0;
	s.ignore_ambient = saved.flags & IGNORE_AMBIENT_COLOR != 0;
	s.color = SpriteColor::new(argb(saved.color));
	s.scale = saved.scale.into();
	s.rotation = saved.rotation.into();
	s.saturation = saved.saturation.into();
	s.offset = (entry.offset.0.into(), entry.offset.1.into());
	s.brush_bits = saved.flags & BRUSH_BITS;
}

/// A unit's sprite as the original saves it.
fn sprite_entry(name: &str, s: &UnitSpriteData, sprites: &Sprites) -> SpriteEntry {
	let look = sprites.with(|r| BrushLook::of(r, &s.brush));
	let mut flags = ALWAYS | s.brush_bits;
	for (on, bit) in [
		(s.visible, VISIBLE),
		(s.flipped, FLIPPED),
		(s.ignore_camera_flip, IGNORE_CAMERA_FLIP),
		(s.ignore_ambient, IGNORE_AMBIENT_COLOR),
	] {
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
			rotation: s.rotation as f32,
			saturation: s.saturation as f32,
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

impl TacticalEntities {
	/// Forget every unit and roster (the original's `World.clear` before a load).
	pub(crate) fn clear_for_load(&self) {
		let mut m = self.machine.borrow_mut();
		m.shakes.clear();
		m.rosters.clear();
		m.units.clear();
		m.by_identity.clear();
	}

	/// Re-create the unit of script `path` saved as `head` + `native` (it joins its
	/// roster with [`Self::add_loaded`]).
	pub(crate) fn load_original(
		&self,
		vm: &SQVM,
		path: &str,
		head: &Head,
		native: &TacticalNative,
	) -> Result<SqStrongRef, String> {
		let id = i64::from(head.id);
		let (unit, idx) = create_unit(&self.machine, vm, path, id)?;
		{
			let mut m = self.machine.borrow_mut();
			if m.ids.peek() <= id {
				m.ids.restore(id + 1);
			}
			m.units[idx].local = (head.pos.0.into(), head.pos.1.into());
		}
		unit.call_method(vm, "onInit", ())?;
		ignore_missing(unit.call_method(vm, "onAfterInit", ()))?;
		let (sprite_class, sprites) = {
			let mut m = self.machine.borrow_mut();
			let rec = &mut m.units[idx];
			rec.initialized = native.game.initialized;
			rec.faction = native.game.faction.into();
			rec.render_callback = native.game.render_callback;
			(m.sprite_class.clone().ok_or("load: sprite class missing")?, m.sprites.clone())
		};
		for entry in &native.game.sprites {
			let name = String::from_utf8_lossy(&entry.name).into_owned();
			let handle = self.machine.borrow().units[idx].sprites.get(&name).cloned();
			if let Some(handle) = handle {
				sprite_class.with_payload(vm, &handle, |s| apply_sprite(s, entry, &sprites));
			}
		}
		Ok(unit)
	}

	/// The rosters as the original saves them (the battle's and the world's temporary
	/// rosters aren't saved).
	pub(crate) fn original_rosters(&self, vm: &SQVM) -> Result<Rosters, String> {
		let units = |id: i64| -> Result<Vec<SavedUnit>, String> {
			let m = self.machine.borrow();
			let sprite_class = m.sprite_class.clone().ok_or("save: sprite class missing")?;
			let mut out = Vec::new();
			for unit in m.rosters.get(&id).into_iter().flatten() {
				let rec = m.record(unit.identity()).ok_or("save: unit not registered")?;
				let mut sprites = Vec::new();
				for (name, handle) in rec.sprites.iter() {
					let entry = sprite_class
						.with_payload(vm, handle, |s| sprite_entry(name, s, &m.sprites))
						.ok_or("save: not a unit sprite")?;
					sprites.push(entry);
				}
				let faction = rec.faction as u8;
				out.push(SavedUnit {
					head: Head {
						id: rec.id as u32,
						pos: { let p = m.world_pos(rec); (p.0 as f32, p.1 as f32) },
						z: 0,
						i14: 0,
						rotation: 0.0,
						flags: 0x0010_2c7d,
						u34: 0,
						children: Vec::new(),
					},
					native: TacticalNative {
						script_hash: class_name_hash(&rec.path) as i32,
						game: GameEntity {
							storing_color: false,
							initialized: rec.initialized,
							render_callback: rec.render_callback,
							sprite_occlusion: false,
							faction,
							faction_bit: if faction == 0 { 0 } else { 1u32 << (faction % 32) },
							sprites,
						},
					},
					script: unit.clone(),
				});
			}
			Ok(out)
		};
		let mut created: Vec<i64> = self.machine.borrow().rosters.keys().copied().filter(|&id| id >= 0).collect();
		created.sort_unstable();
		Ok(Rosters {
			player: units(PLAYER_ROSTER_ID)?,
			guests: units(GUEST_ROSTER_ID)?,
			created: created.into_iter().map(|id| Ok((id as u32, units(id)?))).collect::<Result<_, String>>()?,
		})
	}

	/// Make `roster` exist, empty if it's new.
	pub(crate) fn ensure_roster(&self, roster: Roster) {
		self.machine.borrow_mut().rosters.entry(our_roster_id(roster)).or_default();
	}

	/// Add a loaded unit to `roster` (the original appends it once its record, script
	/// data included, is read).
	pub(crate) fn add_loaded(&self, roster: Roster, unit: SqStrongRef) {
		self.machine.borrow_mut().rosters.entry(our_roster_id(roster)).or_default().push(unit);
	}
}
