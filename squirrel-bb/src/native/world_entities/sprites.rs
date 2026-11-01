//! An entity's named sprites (the original's entity base class, disassembled
//! 2026-10-14): `addSprite` puts a new one on top and `insertSprite` (`0x4a7a60`) below
//! the others; either hands back the sprite already there for a name it has (and the
//! original logs it). `removeSprite` (`0x4a8550`) drops one, keeping the others' order,
//! and does nothing for a name it doesn't have.
//!
//! How they draw: `setSpriteOffset`/`getSpriteOffset`, `setSpriteScaling` and
//! `setSpriteColorization` address a sprite by name. Offsets are world units with y up
//! (a banner at `createVec(90, 80)` sits up and to the right); `getSpriteOffset` of a
//! name the entity doesn't have is `(0, 0)` (`0x4a82b0`; the original logs it). Scripts
//! only ever turn scaling and colorization off, so both default to on; what they
//! change in the drawing is the renderer's business.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Retained, SQVM, SqStrongRef, SqTable};

use super::{EntityMachine, SpriteData, vec_xy};

type Machine = Rc<RefCell<EntityMachine>>;

/// Change the calling entity's sprite `name`. A missing entity or sprite is ignored.
fn with_sprite(m: &Machine, vm: &SQVM, name: &str, f: impl FnOnce(&mut SpriteData)) {
	let target = {
		let machine = m.borrow();
		let class = machine.sprite_class.clone();
		let handle = machine.record(vm.this_identity()).and_then(|rec| rec.sprites.get(name).cloned());
		class.zip(handle)
	};
	if let Some((class, handle)) = target {
		class.with_payload(vm, &handle, f);
		crate::native::world_scene_changes::note();
	}
}

/// `addSprite(name)` (`first`: `insertSprite`) on the calling entity.
pub(super) fn add(m: &Machine, vm: &SQVM, name: String, first: bool) -> Option<SqStrongRef> {
	let (class, owner) = {
		let machine = m.borrow();
		let idx = machine.by_identity.get(&vm.this_identity()).copied();
		if let Some(there) = idx.and_then(|i| machine.entities[i].sprites.get(&name)) {
			return Some(there.clone());
		}
		(machine.sprite_class.clone()?, idx.map(|i| machine.owner(i)))
	};
	let sprite = class.instantiate(vm, SpriteData::fresh(owner));
	let mut machine = m.borrow_mut();
	if let Some(rec) = machine.record_mut(vm.this_identity()) {
		if first {
			rec.sprites.insert_first(name, sprite.clone());
		} else {
			rec.sprites.insert(name, sprite.clone());
		}
	}
	// Its brush's frames start at its first turn (see `sprite_anim`).
	if let Some(owner) = owner {
		machine.moving.note(owner);
	}
	crate::native::world_scene_changes::note();
	Some(sprite)
}

pub(super) fn entity_methods(m: &Machine, vm: &mut SQVM) -> Vec<(&'static str, SqStrongRef)> {
	let insert = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, name: String| add(&m, vm, name, true))
	};
	let remove = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, name: &str| {
			if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
				rec.sprites.remove(&name);
			}
			crate::native::world_scene_changes::note();
		})
	};
	let get_offset = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, name: &str| -> Result<Option<SqStrongRef>, String> {
			let mut offset = (0.0, 0.0);
			with_sprite(&m, vm, &name, |s| offset = s.offset);
			let create_vec = SqTable::root(vm)
				.get::<SqStrongRef>("createVec")
				.ok_or("getSpriteOffset: createVec missing")?;
			create_vec.call_ret::<SqStrongRef, _>(vm, offset)
		})
	};
	let set_offset = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, name: &str, offset: Retained| -> Result<(), String> {
			let offset = vec_xy(vm, &offset.0, "setSpriteOffset")?;
			with_sprite(&m, vm, &name, |s| s.offset = offset);
			Ok(())
		})
	};
	let set_scaling = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, name: &str, on: bool| {
			with_sprite(&m, vm, &name, |s| s.scaling = on);
		})
	};
	let set_colorization = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, name: &str, on: bool| {
			with_sprite(&m, vm, &name, |s| s.colorization = on);
		})
	};
	let set_in_portrait = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, name: &str, on: bool| {
			with_sprite(&m, vm, &name, |s| s.in_portrait = on);
		})
	};
	let set_portrait_offsets = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, on: bool| {
			if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
				rec.portrait_offsets = on;
			}
		})
	};
	vec![
		("setSpriteRenderToTexture", set_in_portrait),
		("setAlwaysApplySpriteOffset", set_portrait_offsets),
		("insertSprite", insert),
		("removeSprite", remove),
		("getSpriteOffset", get_offset),
		("setSpriteOffset", set_offset),
		("setSpriteScaling", set_scaling),
		("setSpriteColorization", set_colorization),
	]
}
