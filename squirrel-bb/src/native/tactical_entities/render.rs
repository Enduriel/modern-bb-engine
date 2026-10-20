//! Drawing a battle, as far as the game logic can tell: each frame `Tactical.render()`
//! runs `onRender` on the units that asked for it (`setRenderCallbackEnabled(true)`) and
//! that the player can see, since the original only draws those. Spiders, ghosts and a
//! few others animate there, and some draw random numbers the first time they are
//! drawn, so this decides when that happens. Headless, everything counts as on screen.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqStrongRef, SqTable};

use super::{TacticalMachine, ignore_missing};
use crate::native::tactical_grid::visibility::PLAYER;

type Machine = Rc<RefCell<TacticalMachine>>;

/// `Time.getVirtualTimeF()`, the clock sprite animations run on.
pub(super) fn virtual_seconds(vm: &SQVM) -> Result<f64, String> {
	let f = SqTable::root(vm)
		.get::<SqTable>("Time")
		.and_then(|t| t.get::<SqStrongRef>("getVirtualTimeF"))
		.ok_or("moveSpriteOffset: no Time.getVirtualTimeF")?;
	Ok(f.call_ret::<Number, _>(vm, ())?.map_or(0.0, |n| n.0))
}

/// The injected unit natives `setRenderCallbackEnabled(on)` and `moveSpriteOffset`.
pub(super) fn unit_natives(vm: &mut SQVM, m: &Machine) -> Vec<(&'static str, SqStrongRef)> {
	let set_enabled = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, on: bool| {
			if let Some(r) = m.borrow_mut().record_mut(vm.this_identity()) {
				r.render_callback = on;
			}
		})
	};
	// moveSpriteOffset(sprite, from, to, duration, start): slides a sprite's offset from
	// `from` to `to` over `duration` seconds of virtual time since `start`, and says
	// whether it has arrived (disassembled, 0x4a8be0): t = (f32 virtual time − start) /
	// duration, arrived at t ≥ 1; a sprite the unit doesn't have counts as arrived.
	// Scripts end animations on it (raising a shield turns the render callback off) and
	// ghosts, ghouls and spiders draw their next jitter target then. Headless only the
	// answer matters; the offset isn't drawn.
	let move_sprite_offset = {
		let m = m.clone();
		vm.make_closure(
			move |vm: &SQVM, sprite: String, _from: Retained, _to: Retained, duration: Number, start: Number| -> Result<bool, String> {
				let has = m.borrow().record(vm.this_identity()).is_some_and(|r| r.sprites.contains_key(&sprite));
				if !has {
					return Ok(true);
				}
				let now = virtual_seconds(vm)? as f32;
				Ok((now - start.0 as f32) / duration.0 as f32 >= 1.0)
			},
		)
	};
	// setVisibleInFogOfWar(on) (`0x4c2c10`): whether the unit is drawn on a tile the
	// player can't see now (desaturated). Actors turn it off, so the player never sees
	// one there.
	let set_visible_in_fog = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, on: bool| {
			if let Some(r) = m.borrow_mut().record_mut(vm.this_identity()) {
				r.visible_in_fog = on;
			}
		})
	};
	vec![
		("setRenderCallbackEnabled", set_enabled),
		("moveSpriteOffset", move_sprite_offset),
		("setVisibleInFogOfWar", set_visible_in_fog),
	]
}

/// `Tactical.render()`.
pub(super) fn tactical_natives(vm: &mut SQVM, m: &Machine) -> Vec<(&'static str, SqStrongRef)> {
	let render = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> Result<(), String> { run_pass(&m, vm) })
	};
	vec![("render", render)]
}

/// One frame's `onRender` calls, in creation order.
fn run_pass(m: &Machine, vm: &SQVM) -> Result<(), String> {
	let drawn: Vec<SqStrongRef> = {
		let machine = m.borrow();
		let Some(grid) = machine.grid.clone() else { return Ok(()) };
		let g = grid.borrow();
		machine
			.units
			.iter()
			.filter(|u| u.render_callback && !u.dead)
			.filter(|u| u.tile.is_some_and(|t| g.tiles[t].vis_mask.contains(PLAYER)))
			.map(|u| u.entity.clone())
			.collect()
	};
	for e in drawn {
		ignore_missing(e.call_method(vm, "onRender", ()))?;
	}
	draw_sprites(m, vm);
	Ok(())
}

/// Draw the units the player sees: what a draw does to their sprites is finish their
/// fades (see `SpriteColor`), which may hide them.
fn draw_sprites(m: &Machine, vm: &SQVM) {
	let now = crate::native::sprite_color::real_time(vm);
	let machine = m.borrow();
	let (Some(grid), Some(class)) = (machine.grid.clone(), machine.sprite_class.clone()) else { return };
	let g = grid.borrow();
	// Only the units noted as moving can have a fade (see `moving_sprites`).
	machine.moving.each(|owner| {
		let seen = |u: &&super::UnitRecord| !u.dead && u.tile.is_some_and(|t| g.tiles[t].vis_mask.contains(PLAYER));
		let Some(u) = machine.owner_record(owner).filter(seen) else { return };
		for (_, handle) in u.sprites.iter() {
			class.with_payload(vm, handle, |s| {
				if s.visible && s.color.draw(now).hide {
					s.visible = false;
				}
			});
		}
	});
}
