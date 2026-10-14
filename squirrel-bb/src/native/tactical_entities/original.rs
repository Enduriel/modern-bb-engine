//! The rosters' share of the original's save layout (see `save::original`): a unit is
//! re-created from its record (`0x4c36f0`): position, then its script (`create`,
//! `onInit`, `onAfterInit`), then its faction and saved sprites, matched by name to
//! the ones `onInit` added. Its script's data follows (the caller runs `onDeserialize`).

use squirrel::{SQVM, SqStrongRef};

use super::{GUEST_ROSTER_ID, PLAYER_ROSTER_ID, TacticalEntities, UnitSpriteData, create_unit, ignore_missing};
use crate::native::Sprites;
use crate::native::save::original::record::{Head, TacticalNative};
use crate::native::save::original::sprite::{FLIPPED, IGNORE_AMBIENT_COLOR, IGNORE_CAMERA_FLIP, SpriteEntry, VISIBLE};
use crate::native::save::original::world::Roster;
use crate::native::sprite_color::SpriteColor;
use crate::native::world_entities::argb;
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
}

impl TacticalEntities {
	/// Forget every unit and roster (the original's `World.clear` before a load).
	pub(crate) fn clear_for_load(&self) {
		let mut m = self.machine.borrow_mut();
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
			m.units[idx].pos = (head.pos.0.into(), head.pos.1.into());
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

	/// Add a loaded unit to `roster` (the original appends it once its record, script
	/// data included, is read).
	pub(crate) fn add_loaded(&self, roster: Roster, unit: SqStrongRef) {
		self.machine.borrow_mut().rosters.entry(our_roster_id(roster)).or_default().push(unit);
	}
}
