//! Drawing a battle, as far as the game logic can tell: each frame `Tactical.render()`
//! runs `onRender` on the units that asked for it (`setRenderCallbackEnabled(true)`) and
//! that the player can see, since the original only draws those. Spiders, ghosts and a
//! few others animate there, and some draw random numbers the first time they are
//! drawn, so this decides when that happens. Headless, everything counts as on screen.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef};

use super::{TacticalMachine, ignore_missing};
use crate::native::tactical_grid::visibility::PLAYER;
use crate::native::tactical_grid::zone::faction_bit;

type Machine = Rc<RefCell<TacticalMachine>>;

/// The injected unit native `setRenderCallbackEnabled(on)`.
pub(super) fn unit_natives(vm: &mut SQVM, m: &Machine) -> Vec<(&'static str, SqStrongRef)> {
	let set_enabled = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, on: bool| {
			if let Some(r) = m.borrow_mut().record_mut(vm.this_identity()) {
				r.render_callback = on;
			}
		})
	};
	vec![("setRenderCallbackEnabled", set_enabled)]
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
			.filter(|u| u.tile.is_some_and(|t| g.tiles[t].vis_mask & faction_bit(PLAYER) != 0))
			.map(|u| u.entity.clone())
			.collect()
	};
	for e in drawn {
		ignore_missing(e.call_method(vm, "onRender", ()))?;
	}
	Ok(())
}
