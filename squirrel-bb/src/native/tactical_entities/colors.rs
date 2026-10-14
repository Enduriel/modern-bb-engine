//! A battle unit's colours as a whole: what darkflight, a teleport, an alp's teleport or
//! a kraken's move do to fade a unit away and back, and undead rising from or sinking
//! into the ground (disassembled 2026-10-14; bound on the entity base class, so the
//! world's entities have the same).
//!
//! - `storeSpriteColors()` / `restoreSpriteColors()` (`0x4a9180`, `0x4a92e0`): keep every
//!   sprite's colours and put them back, as a shake does for its layers; one flag per
//!   unit, shared with shakes, which `isStoringColor()` (`0x4a6c30`) answers. The
//!   kraken's tentacles wait on it after a move.
//! - `fadeTo(color, ms)` (`0x4a9850`), `fadeToStoredColors(ms)` (`0x4a98c0`): every sprite
//!   fades to `color`, or to its kept colour (kept or not; the flag stays).
//! - `fadeIn(ms)`, `fadeOut(ms)`, `setAlpha(alpha)` (`0x4a9930`, `0x4a99c0`, `0x4a9ac0`):
//!   every sprite to full alpha, to clear, or to `alpha` at once.
//! - `moveSpriteScale(sprite, from, to, duration, start)` (`0x4a8d20`): `moveSpriteOffset`
//!   for the sprite's scale.
//! - `copySpritesFrom(entity, names)` (`0x4c3fc0`): the named sprites take the other
//!   unit's brush, colour and saturation (the barber's preview).
//!
//! Fades run on real time, hidden sprites included.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqArray, SqStrongRef};

use super::shaker::{Layers, restore_colors, store_colors};
use super::{TacticalMachine, UnitSpriteData, unit_identity};
use crate::native::color::color_argb;
use crate::native::sprite_color::real_time;

type Machine = Rc<RefCell<TacticalMachine>>;

/// Do `f` to every sprite of the calling unit.
fn each_sprite(m: &Machine, vm: &SQVM, mut f: impl FnMut(&mut UnitSpriteData)) {
	let (class, handles) = {
		let machine = m.borrow();
		let Some(rec) = machine.record(vm.this_identity()) else { return };
		let handles: Vec<SqStrongRef> = rec.sprites.iter().map(|(_, h)| h.clone()).collect();
		(machine.sprite_class.clone(), handles)
	};
	let Some(class) = class else { return };
	for h in handles {
		class.with_payload(vm, &h, &mut f);
	}
}

/// The calling unit's record.
fn unit(m: &Machine, vm: &SQVM) -> Option<usize> {
	m.borrow().by_identity.get(&vm.this_identity()).copied()
}

/// `storeSpriteColors` and `restoreSpriteColors` (each a single Sqrat overload).
pub(super) fn store_and_restore(vm: &mut SQVM, m: &Machine) -> (SqStrongRef, SqStrongRef) {
	let store = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| {
			if let Some(idx) = unit(&m, vm) {
				store_colors(&mut m.borrow_mut(), vm, idx, Layers::All);
			}
		})
	};
	let restore = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| {
			if let Some(idx) = unit(&m, vm) {
				let mut machine = m.borrow_mut();
				if machine.units[idx].colors_stored {
					restore_colors(&mut machine, vm, idx, Layers::All);
				}
			}
		})
	};
	(store, restore)
}

/// The injected unit natives.
pub(super) fn unit_natives(vm: &mut SQVM, m: &Machine) -> Vec<(&'static str, SqStrongRef)> {
	let is_storing = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| m.borrow().record(vm.this_identity()).is_some_and(|r| r.colors_stored))
	};
	let fade_to = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, color: Retained, ms: Number| -> Result<(), String> {
			let argb = color_argb(vm, &color.0).ok_or("fadeTo: not a color")?;
			let now = real_time(vm);
			each_sprite(&m, vm, |s| s.color.fade_to(argb, ms.0 as i64, now, false));
			Ok(())
		})
	};
	let fade_to_stored = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, ms: Number| {
			let now = real_time(vm);
			each_sprite(&m, vm, |s| s.color.fade_to(s.stored.color, ms.0 as i64, now, false));
		})
	};
	let fade_in = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, ms: Number| {
			let now = real_time(vm);
			each_sprite(&m, vm, |s| s.color.fade_in(ms.0 as i64, now));
		})
	};
	let fade_out = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, ms: Number| {
			let now = real_time(vm);
			each_sprite(&m, vm, |s| s.color.fade_out(ms.0 as i64, now, false));
		})
	};
	let set_alpha = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, alpha: Number| each_sprite(&m, vm, |s| s.color.snap_alpha(alpha.0 as i64)))
	};
	// moveSpriteScale(sprite, from, to, duration, start): the sprite's scale from `from`
	// to `to` over `duration` seconds of virtual time since `start`; whether it's there
	// (a sprite the unit lacks is).
	let move_sprite_scale = {
		let m = m.clone();
		vm.make_closure(
			move |vm: &SQVM, sprite: String, from: Number, to: Number, duration: Number, start: Number| -> Result<bool, String> {
				let (class, handle) = {
					let machine = m.borrow();
					let handle = machine.record(vm.this_identity()).and_then(|r| r.sprites.get(&sprite).cloned());
					(machine.sprite_class.clone(), handle)
				};
				let (Some(class), Some(handle)) = (class, handle) else { return Ok(true) };
				let now = super::render::virtual_seconds(vm)? as f32;
				let t = (now - start.0 as f32) / duration.0 as f32;
				let (from, to) = (from.0 as f32, to.0 as f32);
				let (scale, done) = if t >= 1.0 { (to, true) } else { (from + (to - from) * t, false) };
				class.with_payload(vm, &handle, |s| s.scale = f64::from(scale));
				Ok(done)
			},
		)
	};
	// copySpritesFrom(entity, names): each named sprite both units have takes the other's
	// brush, colour and saturation.
	let copy_sprites_from = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, other: Retained, names: Retained| -> Result<(), String> {
			let names: Vec<String> = names.0.get_value::<SqArray>(vm).map(|a| a.iter::<String>().collect()).unwrap_or_default();
			let (class, sprites, pairs) = {
				let machine = m.borrow();
				let (Some(to), Some(from)) = (machine.record(vm.this_identity()), machine.record(unit_identity(vm, &other.0))) else {
					return Ok(());
				};
				let pairs: Vec<(SqStrongRef, SqStrongRef)> = names
					.iter()
					.filter_map(|n| Some((to.sprites.get(n)?.clone(), from.sprites.get(n)?.clone())))
					.collect();
				(machine.sprite_class.clone(), machine.sprites.clone(), pairs)
			};
			let Some(class) = class else { return Ok(()) };
			for (to, from) in pairs {
				let Some((brush, color, saturation)) = class.with_payload(vm, &from, |s| (s.brush.clone(), s.color.color, s.saturation)) else {
					continue;
				};
				class.with_payload(vm, &to, |s| {
					s.spin.rebrush(&sprites, &s.brush, &brush);
					s.brush_bits = sprites.brush_bits(&brush, s.brush_bits);
					s.brush = brush;
					s.color.color = color;
					s.saturation = saturation;
				});
			}
			Ok(())
		})
	};
	vec![
		("isStoringColor", is_storing),
		("fadeTo", fade_to),
		("fadeToStoredColors", fade_to_stored),
		("fadeIn", fade_in),
		("fadeOut", fade_out),
		("setAlpha", set_alpha),
		("moveSpriteScale", move_sprite_scale),
		("copySpritesFrom", copy_sprites_from),
	]
}
