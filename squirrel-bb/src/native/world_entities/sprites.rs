//! How an entity's sprites draw: the entity natives `setSpriteOffset`,
//! `setSpriteScaling` and `setSpriteColorization`, which address a sprite by name.
//! Offsets are world units with y up (a banner at `createVec(90, 80)` sits up and to
//! the right). Scripts only ever turn scaling and colorization off, so both default
//! to on; what they change in the drawing is the renderer's business.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Retained, SQVM, SqStrongRef};

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
	}
}

pub(super) fn entity_methods(m: &Machine, vm: &mut SQVM) -> Vec<(&'static str, SqStrongRef)> {
	let set_offset = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, name: String, offset: Retained| {
			let offset = vec_xy(vm, &offset.0);
			with_sprite(&m, vm, &name, |s| s.offset = offset);
		})
	};
	let set_scaling = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, name: String, on: bool| {
			with_sprite(&m, vm, &name, |s| s.scaling = on);
		})
	};
	let set_colorization = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, name: String, on: bool| {
			with_sprite(&m, vm, &name, |s| s.colorization = on);
		})
	};
	vec![
		("setSpriteOffset", set_offset),
		("setSpriteScaling", set_scaling),
		("setSpriteColorization", set_colorization),
	]
}
