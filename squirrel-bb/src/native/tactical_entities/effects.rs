//! The battle's visual effect spawners (`Tactical.spawn…Effect`). Headless they draw
//! nothing; what scripts get back is kept: a sprite from `spawnSpriteEffect`, one sprite
//! per layer from `spawnHeadEffect`, and from `spawnProjectileEffect` its flight time in
//! ms, which scripts schedule the hit with. That time is the original's (disassembled):
//! `trunc(min(0.75, d · 0.0007) · timeScale · 1000)`, `d` the pixel distance between the
//! tiles with the target 5 px lower.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Native, Number, Retained, SQVM, SqArray, SqStrongRef, Variadic};

use super::{TacticalMachine, UnitSpriteData};
use crate::native::tactical_grid::TileHandle;

type Machine = Rc<RefCell<TacticalMachine>>;

fn sprite(m: &Machine, vm: &SQVM) -> Option<SqStrongRef> {
	let class = m.borrow().sprite_class.clone()?;
	Some(class.instantiate(vm, UnitSpriteData::fresh()))
}

/// Flight time (ms) of a projectile from tile `from` to tile `to`.
fn projectile_ms(from: (f64, f64), to: (f64, f64), time_scale: f32) -> i64 {
	let dx = (to.0 - from.0) as f32;
	let dy = (to.1 as f32 + 5.0) - from.1 as f32;
	let d = (dx * dx + dy * dy).sqrt();
	((0.75f32).min(d * 0.0007) * time_scale * 1000.0) as i64
}

pub(super) fn natives(vm: &mut SQVM, m: &Machine) -> Vec<(&'static str, SqStrongRef)> {
	let none = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let sprite_effect = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, Variadic(_a): Variadic<Retained>| sprite(&m, vm))
	};
	// spawnHeadEffect(tile, layers, offset, rotation, bloodPool): a sprite per layer.
	let head_effect = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, _tile: Retained, layers: SqArray, Variadic(_rest): Variadic<Retained>| -> SqStrongRef {
			let mut out = SqArray::new(vm);
			for _ in 0..layers.len() {
				out.push(sprite(&m, vm));
			}
			out.to_strong()
		})
	};
	// spawnProjectileEffect(brush, from, to, speed, timeScale, rotated, flip) -> ms.
	let projectile = vm.make_closure(
		|_brush: Retained, from: Native<TileHandle>, to: Native<TileHandle>, _speed: Number, time_scale: Number, Variadic(_rest): Variadic<Retained>| -> i64 {
			let pos = |t: &TileHandle| {
				let g = t.grid.borrow();
				g.tiles[t.id].world_pos()
			};
			projectile_ms(pos(&from.0), pos(&to.0), time_scale.0 as f32)
		},
	);
	vec![
		("spawnIconEffect", none.clone()),
		("spawnAttackEffect", none.clone()),
		("spawnPoolEffect", none),
		("spawnSpriteEffect", sprite_effect),
		("spawnHeadEffect", head_effect),
		("spawnProjectileEffect", projectile),
	]
}

#[cfg(test)]
mod tests {
	use super::projectile_ms;

	#[test]
	fn projectile_time_caps_at_three_quarters_of_a_second() {
		// Adjacent columns: ~120 px → 84 ms (truncated); far away caps at 750 ms.
		assert_eq!(projectile_ms((80.0, 40.0), (200.0, 80.0), 1.0), (((120.0f32 * 120.0 + 45.0 * 45.0).sqrt() * 0.0007) * 1000.0) as i64);
		assert_eq!(projectile_ms((0.0, 0.0), (5000.0, 0.0), 1.0), 750);
	}
}
