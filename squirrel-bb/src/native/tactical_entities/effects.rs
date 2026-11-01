//! The battle's visual effect spawners (`Tactical.spawn…Effect`). They record into
//! the battle's effects (see `battle_effects`), which the renderer draws; what scripts
//! get back is kept: a sprite from `spawnSpriteEffect`, one sprite per layer from
//! `spawnHeadEffect`, and from `spawnProjectileEffect` its flight time in ms, which
//! scripts schedule the hit with. That time is the original's (disassembled):
//! `trunc(min(0.75, d · 0.0007) · timeScale · 1000)`, `d` the pixel distance between the
//! tiles with the target 5 px lower. An unknown brush shows nothing.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Native, Number, Retained, SQVM, SqArray, SqStrongRef, Variadic};

use super::{TacticalMachine, UnitSpriteData};
use crate::native::battle_effects::{Head, Projectile, flight_time};
use crate::native::sprite_color::virtual_time;
use crate::native::tactical_grid::{TileHandle, heads};

type Machine = Rc<RefCell<TacticalMachine>>;

fn sprite(m: &Machine, vm: &SQVM) -> Option<SqStrongRef> {
	let class = m.borrow().sprite_class.clone()?;
	Some(class.instantiate(vm, UnitSpriteData::fresh(None)))
}

/// Flight time (ms) of a projectile from tile `from` to tile `to`.
fn projectile_ms(from: (f64, f64), to: (f64, f64), time_scale: f32) -> i64 {
	let from = (from.0 as f32, from.1 as f32);
	let to = (to.0 as f32, to.1 as f32);
	(flight_time(from, to, time_scale) * 1000.0) as i64
}

/// A tile's position and index.
fn tile_of(t: &TileHandle) -> ((f32, f32), usize) {
	let (x, y) = t.grid.borrow().tiles[t.id].world_pos();
	((x as f32, y as f32), t.id)
}


/// Argument `i` as a number (0 if missing).
fn num(vm: &SQVM, args: &[Retained], i: usize) -> f32 {
	args.get(i).and_then(|a| a.0.get_value::<Number>(vm)).map_or(0.0, |n| n.0 as f32)
}

/// Argument `i` as a `vec2f`'s `X`, `Y` (0, 0 if missing).
fn vec2(vm: &SQVM, args: &[Retained], i: usize, native: &str) -> Result<(f32, f32), String> {
	args.get(i).map_or(Ok((0.0, 0.0)), |v| crate::native::vec::position_f32(vm, &v.0, native))
}

/// `brush`'s rotation speed, or `None` for an unknown brush.
fn brush_speed(m: &Machine, brush: &str) -> Option<f32> {
	m.borrow().sprites.with(|r| r.get(brush).map(|(_, s)| s.rotation_speed))
}

pub(super) fn natives(vm: &mut SQVM, m: &Machine) -> Vec<(&'static str, SqStrongRef)> {
	// spawnIconEffect(brush, tile, offsetX, offsetY, scale, fadeIn, stay, fadeOut, movement)
	let icon = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, brush: String, tile: Native<TileHandle>, Variadic(a): Variadic<Retained>| -> Result<(), String> {
			let movement = vec2(vm, &a, 6, "spawnIconEffect")?;
			let (Some(_), Some(effects)) = (brush_speed(&m, &brush), m.borrow().effects.clone()) else { return Ok(()) };
			let ((x, y), id) = tile_of(&tile.0);
			let phases = crate::native::battle_effects::Phases {
				fade_in: num(vm, &a, 3),
				stay: num(vm, &a, 4),
				fade_out: num(vm, &a, 5),
			};
			let pos = (x + num(vm, &a, 0), y + num(vm, &a, 1));
			effects.borrow_mut().spawn_icon(brush, id, pos, num(vm, &a, 2), phases, movement, virtual_time(vm));
			Ok(())
		})
	};
	// spawnAttackEffect(brush, tile, offsetX, offsetY, fadeIn, stay, fadeOut, movement,
	// switchMs, movementAfter, inFront)
	let attack = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, brush: String, tile: Native<TileHandle>, Variadic(a): Variadic<Retained>| -> Result<(), String> {
			let moves = [vec2(vm, &a, 5, "spawnAttackEffect")?, vec2(vm, &a, 7, "spawnAttackEffect")?];
			let (Some(_), Some(effects)) = (brush_speed(&m, &brush), m.borrow().effects.clone()) else { return Ok(()) };
			let ((x, y), _) = tile_of(&tile.0);
			let phases = crate::native::battle_effects::Phases {
				fade_in: num(vm, &a, 2),
				stay: num(vm, &a, 3),
				fade_out: num(vm, &a, 4),
			};
			let pos = (x + num(vm, &a, 0), y + num(vm, &a, 1));
			effects.borrow_mut().spawn_attack(brush, pos, phases, moves, num(vm, &a, 6), virtual_time(vm));
			Ok(())
		})
	};
	// spawnPoolEffect(brush, tile, alpha, scale, flags)
	let pool = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, brush: String, tile: Native<TileHandle>, Variadic(a): Variadic<Retained>| {
			let (Some(_), Some(effects)) = (brush_speed(&m, &brush), m.borrow().effects.clone()) else { return };
			let (_, id) = tile_of(&tile.0);
			let flags = a.get(2).and_then(|f| f.0.get_value::<Number>(vm)).map_or(0, |n| n.0 as i64);
			effects.borrow_mut().spawn_pool(id, brush, num(vm, &a, 0), num(vm, &a, 1), flags, virtual_time(vm));
		})
	};
	// spawnSpriteEffect(brush, color, tile, offsetX, offsetY, scaleFrom, scaleTo, hold,
	// fadeIn, fadeOut): the effect, and a sprite for the script.
	let sprite_effect = {
		let m = m.clone();
		vm.make_closure(
			move |vm: &SQVM, brush: String, color: Retained, tile: Native<TileHandle>, Variadic(a): Variadic<Retained>| {
				if let (Some(_), Some(effects)) = (brush_speed(&m, &brush), m.borrow().effects.clone()) {
					let ((x, y), _) = tile_of(&tile.0);
					let argb = crate::native::color::color_argb(vm, &color.0).unwrap_or(0xffff_ffff);
					let effect = crate::native::battle_effects::SpriteEffect::new(
						brush,
						(x + num(vm, &a, 0), y + num(vm, &a, 1)),
						argb,
						(num(vm, &a, 2), num(vm, &a, 3)),
						num(vm, &a, 4),
						num(vm, &a, 5),
						num(vm, &a, 6),
						virtual_time(vm),
					);
					effects.borrow_mut().spawn_sprite(effect);
				}
				sprite(&m, vm)
			},
		)
	};
	// spawnHeadEffect(tile, layers, offset, rotation, bloodPool): a sprite per layer
	// (brush names), which the script colours; the head flies off with them.
	let head_effect = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, tile: Native<TileHandle>, layers: SqArray, Variadic(a): Variadic<Retained>| -> Result<SqStrongRef, String> {
			let offset = vec2(vm, &a, 0, "spawnHeadEffect")?;
			let rotation = num(vm, &a, 1);
			let (class, sprites) = {
				let mm = m.borrow();
				(mm.sprite_class.clone(), mm.sprites.clone())
			};
			let mut out = SqArray::new(vm);
			let mut handles = Vec::new();
			for i in 0..layers.len() {
				let Some(handle) = sprite(&m, vm) else { continue };
				if let (Some(class), Some(name)) = (&class, layers.get::<String>(i)) {
					class.with_payload(vm, &handle, |s| {
						s.brush = sprites.resolve_battle(&name);
						s.brush_bits = sprites.brush_bits(&s.brush, s.brush_bits);
						s.rotation = f64::from(rotation);
					});
				}
				handles.push(handle.clone());
				out.push(handle);
			}
			let effects = m.borrow().effects.clone();
			if let Some(effects) = effects {
				let ((x, y), id) = tile_of(&tile.0);
				let pool = a.get(2).and_then(|p| p.0.get_value::<String>(vm)).filter(|p| brush_speed(&m, p).is_some());
				// Where it lands, and the mark it leaves there (see `heads`), before the
				// effects are borrowed: having to land at home clears the tile's heads.
				let crt = effects.borrow().crt();
				let land = heads::landing_tile(&tile.0.grid, vm, id, || crt.rand_default());
				heads::mark_head(&tile.0.grid, vm, land);
				let rng = effects.borrow().rng();
				let head = Head::new(&rng, id, (x, y), land, handles, pool, offset, rotation, virtual_time(vm));
				effects.borrow_mut().spawn_head(head);
			}
			Ok(out.to_strong())
		})
	};
	// spawnProjectileEffect(brush, from, to, scale, timeScale, rotated, flip) -> ms.
	let projectile = {
		let m = m.clone();
		vm.make_closure(
			move |vm: &SQVM, brush: String, from: Native<TileHandle>, to: Native<TileHandle>, scale: Number, time_scale: Number, Variadic(a): Variadic<Retained>| -> i64 {
				let ((fx, fy), _) = tile_of(&from.0);
				let ((tx, ty), _) = tile_of(&to.0);
				let ms = projectile_ms((f64::from(fx), f64::from(fy)), (f64::from(tx), f64::from(ty)), time_scale.0 as f32);
				if let (Some(speed), Some(effects)) = (brush_speed(&m, &brush), m.borrow().effects.clone()) {
					let flag = |i: usize| a.get(i).and_then(|v| v.0.get_value::<bool>(vm)).unwrap_or(false);
					let projectile = Projectile::new(
						brush,
						(fx, fy),
						(tx, ty),
						virtual_time(vm),
						scale.0 as f32,
						time_scale.0 as f32,
						flag(0),
						flag(1),
						speed,
					);
					effects.borrow_mut().spawn_projectile(projectile);
				}
				ms
			},
		)
	};
	vec![
		("spawnIconEffect", icon),
		("spawnAttackEffect", attack),
		("spawnPoolEffect", pool),
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
