//! The shaker, `Tactical.getShaker()` (a list at `Tactical+0x1cc`; disassembled
//! 2026-10-14): the short moves a unit makes in battle. It hops when the player clicks it
//! while it can act, recoils from a hit, dodges a miss, and lunges at its target.
//!
//! - `shake(entity, tile, type)` (`0x4b68f0`) and `shake(entity, tile, type, color,
//!   highlight, factor, saturation, layers, recoverMult)` (`0x4b6a30`) start a shake of
//!   the unit relative to `tile`. A unit that is already shaking ignores it.
//! - `cancel(entity)` (`0x4b7370`) puts the unit back where it started, at once.
//! - Each `Tactical.update()` (`0x4b7450`, after the navigator's update) moves every
//!   shaking unit. Its whole position goes out linearly over the type's `out` seconds of
//!   virtual time, then back over `back` seconds, so a paused battle freezes it.
//! - The colour arguments flash the unit's sprites named in `layers`: their `Highlight`
//!   takes `highlight` at once, `HighlightFactor` ramps up to
//!   `min(factor × recoverMult, 1)` and back, and `Color` and `Saturation` ramp to
//!   `color` and `saturation`. All of them are put back at the end.
//! - A dodge (type 4) turns away 45° to a random side: one draw of the C runtime's
//!   `rand()`, the stream 0-argument `Math.rand()` and pools draw from.
//! - `Tactical.clearScene()` (`0x4b76f0`) puts every shaking unit back and ends its shake.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{ClassBuilder, Number, Retained, SQVM, SqArray, SqStrongRef, SqTable, StackArgs, This, Variadic};

use super::{TacticalMachine, UnitSpriteData, unit_identity};
use crate::native::color::color_argb;
use crate::native::overload::{Overload, overload_key};
use crate::native::tactical_grid;

type Machine = Rc<RefCell<TacticalMachine>>;

/// A shake in progress (the original's 0x50-byte record).
pub(super) struct Shake {
	/// The unit's record.
	unit: usize,
	/// Virtual seconds when it started.
	start: f32,
	/// Where the unit stood then.
	orig: (f32, f32),
	dir: (f32, f32),
	amp: (f32, f32),
	out: f32,
	back: f32,
	/// Coming back (the second phase).
	backing: bool,
	/// `color`, unless white (no colour change).
	color: Option<u32>,
	highlight: u32,
	/// `min(factor × recoverMult, 1)`, unless `factor` is 0 (no flash).
	factor: Option<f32>,
	/// `saturation`, unless 1 (no change).
	saturation: Option<f32>,
	/// The sprites the colours apply to: bit `i % 32` for the unit's `i`th sprite.
	mask: u32,
}

/// The colours a sprite's slot keeps (`slot+0x2c..+0x38`) to put back after a shake, or
/// for `restoreSpriteColors`. A slot always has them (white, none, 0, 1 until first
/// stored); a restore leaves them.
#[derive(Clone, Copy)]
pub(super) struct Stored {
	pub color: u32,
	highlight: u32,
	factor: f64,
	saturation: f64,
}

impl Default for Stored {
	fn default() -> Self {
		Stored {
			color: 0xffff_ffff,
			highlight: 0,
			factor: 0.0,
			saturation: 1.0,
		}
	}
}

/// Which of a unit's sprites a store or restore covers.
#[derive(Clone, Copy)]
pub(super) enum Layers {
	/// All of them (`storeSpriteColors`, `restoreSpriteColors`).
	All,
	/// The `i`th sprite for bit `i % 32` (a shake's layers).
	Mask(u32),
}

/// The arguments of a shake.
struct Args {
	kind: i64,
	color: u32,
	highlight: u32,
	factor: f32,
	saturation: f32,
	layers: Vec<String>,
	recover: f32,
}

/// The shaker's payload: the entity host's records.
#[derive(Clone)]
pub(super) struct ShakerHandle(pub Machine);

/// `Time.getVirtualTimeF()` as the original reads it (`Time+0x20` cast to f32).
fn now(vm: &SQVM) -> Result<f32, String> {
	super::render::virtual_seconds(vm).map(|s| s as f32)
}

fn normalize((x, y): (f32, f32)) -> Option<(f32, f32)> {
	let len = (x * x + y * y).sqrt();
	(len != 0.0).then(|| (x / len, y / len))
}

/// The shaker class (`Tactical.getShaker()`'s instance).
pub(super) fn build_class(vm: &mut SQVM) -> squirrel::SqClass<ShakerHandle> {
	ClassBuilder::<ShakerHandle>::new("TacticalShaker")
		.method(
			&overload_key("shake", 3),
			|s: &mut ShakerHandle, vm: &SQVM, entity: Retained, tile: Retained, kind: Number| -> Result<(), String> {
				let args = Args {
					kind: kind.0 as i64,
					color: 0xffff_ffff,
					highlight: 0,
					factor: 0.0,
					saturation: 1.0,
					layers: Vec::new(),
					recover: 1.0,
				};
				shake(&s.0, vm, &entity.0, &tile.0, args)
			},
		)
		.method(&overload_key("shake", 9), |s: &mut ShakerHandle, vm: &SQVM, args: Variadic<Retained>| -> Result<(), String> {
			let a = &args.0;
			if a.len() != 9 {
				return Err("No overload matching this argument list found".into());
			}
			let number = |i: usize| a[i].0.get_value::<Number>(vm).map(|n| n.0 as f32).ok_or_else(|| format!("shake: argument {} is not a number", i + 1));
			let color = |i: usize| color_argb(vm, &a[i].0).ok_or_else(|| format!("shake: argument {} is not a color", i + 1));
			let layers = a[7]
				.0
				.get_value::<SqArray>(vm)
				.map(|l| l.iter::<String>().collect())
				.ok_or("shake: argument 8 is not an array")?;
			let args = Args {
				kind: number(2)? as i64,
				color: color(3)?,
				highlight: color(4)?,
				factor: number(5)?,
				saturation: number(6)?,
				layers,
				recover: number(8)?,
			};
			shake(&s.0, vm, &a[0].0, &a[1].0, args)
		})
		.native_method("shake", {
			let overload = Overload::new("shake");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		})
		.method("cancel", |s: &mut ShakerHandle, vm: &SQVM, entity: Retained| {
			let identity = unit_identity(vm, &entity.0);
			cancel(&s.0, vm, identity);
		})
		.build(vm)
}

/// `shake(...)`: start one, unless the unit is already shaking.
fn shake(m: &Machine, vm: &SQVM, entity: &SqStrongRef, tile: &SqStrongRef, a: Args) -> Result<(), String> {
	let identity = unit_identity(vm, entity);
	let start = now(vm)?;
	let mut machine = m.borrow_mut();
	let Some(&unit) = machine.by_identity.get(&identity) else { return Ok(()) };
	if machine.shakes.iter().any(|s| s.unit == unit) {
		return Ok(());
	}
	let grid = machine.grid.clone().ok_or("shake: no battle map")?;
	let (src, own) = {
		let g = grid.borrow();
		let src = tactical_grid::tile_id_of(&grid, vm, tile).and_then(|id| g.tiles.get(id)).map(|t| t.world_pos());
		let rec = &machine.units[unit];
		let own = rec.tile.and_then(|id| g.tiles.get(id)).map_or(rec.pos, |t| t.world_pos());
		(src.ok_or("shake: not a tile")?, own)
	};
	let (src, own) = ((src.0 as f32, src.1 as f32), (own.0 as f32, own.1 as f32));
	let away = (own.0 - src.0, own.1 - src.1);
	let toward = (src.0 - own.0, src.1 - own.1);
	// Types 2 to 6 point at or away from the tile, and move up by ampX's amount; the
	// small hit's (3) shares the hit's tail, which moves it up by 7 (a quirk).
	let pointed = |v: (f32, f32), amp: (f32, f32), up: f32| match normalize(v) {
		Some(dir) => (dir, if dir.1 > 0.0 { (amp.0, up) } else { amp }),
		None => ((0.0, -1.0), amp),
	};
	let (dir, amp, out, back) = match a.kind {
		0 => ((0.0, 0.0), (0.0, 0.0), 0.08, 0.4),
		1 => ((0.0, 1.0), (0.0, 7.0), 0.4, 0.4),
		2 => {
			let (dir, amp) = pointed(away, (7.0, 6.0), 7.0);
			(dir, amp, 0.08, 0.4 * a.recover)
		}
		3 => {
			let (dir, amp) = pointed(away, (3.0, 2.0), 7.0);
			(dir, amp, 0.05, 0.25)
		}
		4 => {
			// Away, turned 45° to a random side (`0x4b6f20`).
			let crt = machine.effects.as_ref().map(|e| e.borrow().crt()).ok_or("shake: no random stream")?;
			let (dx, dy) = away;
			let v = if crt.rand_default() % 100 < 50 { (dx - dy, dy + dx) } else { (dx + dy, dy - dx) };
			let (dir, amp) = pointed(v, (7.0, 6.0), 7.0);
			(dir, amp, 0.2, 0.35)
		}
		5 => {
			let (dir, amp) = pointed(toward, (3.5, 3.0), 3.5);
			(dir, amp, 0.08, 0.4)
		}
		6 => {
			let (dir, amp) = pointed(toward, (1.75, 1.5), 1.75);
			(dir, amp, 0.08, 0.4)
		}
		7 => ((0.0, 1.0), (0.0, 14.0), 0.1, 0.2),
		_ => ((0.0, 0.0), (0.0, 0.0), 0.0, 0.0),
	};
	let rec = &machine.units[unit];
	let mut s = Shake {
		unit,
		start,
		orig: (rec.pos.0 as f32, rec.pos.1 as f32),
		dir,
		amp,
		out,
		back,
		backing: false,
		color: (a.color != 0xffff_ffff).then_some(a.color),
		highlight: a.highlight,
		factor: (a.factor != 0.0).then(|| (a.factor * a.recover).min(1.0)),
		saturation: (a.saturation != 1.0).then_some(a.saturation),
		mask: 0,
	};
	if s.color.is_some() || s.factor.is_some() || s.saturation.is_some() {
		s.mask = layer_mask(&machine, unit, &a.layers);
		store_colors(&mut machine, vm, unit, Layers::Mask(s.mask));
	}
	machine.shakes.push(s);
	Ok(())
}

/// The bits of the unit's sprites named in `layers` (`0x4a9560`: the `i`th sprite's bit
/// is `i % 32`; unknown names add none).
fn layer_mask(m: &TacticalMachine, unit: usize, layers: &[String]) -> u32 {
	let names: Vec<&String> = m.units[unit].sprites.iter().map(|(n, _)| n).collect();
	layers
		.iter()
		.filter_map(|l| names.iter().position(|n| *n == l))
		.fold(0, |mask, i| mask | 1 << (i % 32))
}

/// The unit's sprites in `layers`, with their handles.
fn masked(m: &TacticalMachine, unit: usize, layers: Layers) -> Vec<SqStrongRef> {
	m.units[unit]
		.sprites
		.iter()
		.enumerate()
		.filter(|(i, _)| match layers {
			Layers::All => true,
			Layers::Mask(mask) => mask & 1 << (i % 32) != 0,
		})
		.map(|(_, (_, h))| h.clone())
		.collect()
}

fn with_sprite<R>(m: &TacticalMachine, vm: &SQVM, handle: &SqStrongRef, f: impl FnOnce(&mut UnitSpriteData) -> R) -> Option<R> {
	m.sprite_class.as_ref()?.with_payload(vm, handle, f)
}

/// Keep the colours of the sprites in `layers` (`0x4a90c0`; all: `0x4a9180`), once per
/// unit until they are put back: the unit's colours-stored flag, which
/// `isStoringColor()` answers, is shared by shakes and the scripts' stores.
pub(super) fn store_colors(m: &mut TacticalMachine, vm: &SQVM, unit: usize, layers: Layers) {
	if m.units[unit].colors_stored {
		return;
	}
	for h in masked(m, unit, layers) {
		with_sprite(m, vm, &h, |s| {
			if !s.is_stored {
				s.stored = Stored {
					color: s.color.color,
					highlight: s.highlight,
					factor: s.highlight_factor,
					saturation: s.saturation,
				};
				s.is_stored = true;
			}
		});
	}
	m.units[unit].colors_stored = true;
}

/// Put back the colours of the sprites in `layers` that were kept (`0x4a9230`; all:
/// `0x4a92e0`). A fade in progress goes on from the colour put back.
pub(super) fn restore_colors(m: &mut TacticalMachine, vm: &SQVM, unit: usize, layers: Layers) {
	for h in masked(m, unit, layers) {
		with_sprite(m, vm, &h, |s| {
			if s.is_stored {
				let st = s.stored;
				s.color.color = st.color;
				s.highlight = st.highlight;
				s.highlight_factor = st.factor;
				s.saturation = st.saturation;
				s.is_stored = false;
			}
		});
	}
	m.units[unit].colors_stored = false;
}

/// One channel-wise step from `from` to `to` (`0x4a65f0`).
fn lerp_color(from: u32, to: u32, t: f32) -> u32 {
	if from == to {
		return from;
	}
	(0..4).fold(0, |c, i| {
		let (a, b) = ((from >> (i * 8)) & 0xff, (to >> (i * 8)) & 0xff);
		let v = ((a as f32 / 255.0 * (1.0 - t) + b as f32 / 255.0 * t) * 255.0) as u32;
		c | (v & 0xff) << (i * 8)
	})
}

/// The unit `t` of the way out.
fn apply(m: &mut TacticalMachine, vm: &SQVM, s: &Shake, t: f32) {
	m.units[s.unit].pos = (f64::from(s.orig.0 + s.dir.0 * s.amp.0 * t), f64::from(s.orig.1 + s.dir.1 * s.amp.1 * t));
	if s.color.is_none() && s.factor.is_none() && s.saturation.is_none() {
		return;
	}
	for h in masked(m, s.unit, Layers::Mask(s.mask)) {
		with_sprite(m, vm, &h, |sp| {
			let st = sp.stored;
			if let Some(c) = s.color {
				sp.color.color = lerp_color(st.color, c, t);
			}
			if let Some(f) = s.factor {
				sp.highlight = s.highlight;
				sp.highlight_factor = f64::from((1.0 - t) * st.factor as f32 + f * t);
			}
			if let Some(sat) = s.saturation {
				sp.saturation = f64::from((1.0 - t) * st.saturation as f32 + sat * t);
			}
		});
	}
}

/// Put the shaken unit back where it started, with its colours.
fn end(m: &mut TacticalMachine, vm: &SQVM, s: &Shake) {
	if s.color.is_some() || s.factor.is_some() || s.saturation.is_some() {
		restore_colors(m, vm, s.unit, Layers::Mask(s.mask));
	}
	m.units[s.unit].pos = (f64::from(s.orig.0), f64::from(s.orig.1));
}

/// Set `Tactical.getShaker()`, and move the shakes in `Tactical.update()` (after what it
/// does already: the navigator's update) and end them in `Tactical.clearScene()`.
pub(super) fn install(m: &Machine, vm: &mut SQVM) {
	let class = build_class(vm);
	let shaker = class.instantiate(vm, ShakerHandle(m.clone()));
	let get = vm.make_closure(move || shaker.clone());
	fn tactical(vm: &SQVM) -> Option<SqTable<'_>> {
		SqTable::root(vm).get::<SqTable>("Tactical")
	}
	let inner_update = tactical(vm).and_then(|t| t.get::<SqStrongRef>("update"));
	let inner_clear = tactical(vm).and_then(|t| t.get::<SqStrongRef>("clearScene"));
	let update = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
			if let Some(inner) = &inner_update {
				inner.call(vm, ())?;
			}
			update(&m, vm)
		})
	};
	let clear_scene = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
			clear(&m, vm);
			match &inner_clear {
				Some(inner) => inner.call(vm, ()),
				None => Ok(()),
			}
		})
	};
	if let Some(mut t) = tactical(vm) {
		t.set("getShaker", get);
		t.set("update", update);
		t.set("clearScene", clear_scene);
	}
}

/// `cancel(entity)`.
pub(super) fn cancel(m: &Machine, vm: &SQVM, identity: i64) {
	let mut machine = m.borrow_mut();
	let Some(&unit) = machine.by_identity.get(&identity) else { return };
	if let Some(i) = machine.shakes.iter().position(|s| s.unit == unit) {
		let s = machine.shakes.remove(i);
		end(&mut machine, vm, &s);
	}
}

/// `Tactical.clearScene()`'s share: every shaking unit back, no shakes.
pub(super) fn clear(m: &Machine, vm: &SQVM) {
	let mut machine = m.borrow_mut();
	for s in std::mem::take(&mut machine.shakes) {
		end(&mut machine, vm, &s);
	}
}

/// One `Tactical.update()`: every shake a frame on.
pub(super) fn update(m: &Machine, vm: &SQVM) -> Result<(), String> {
	if m.borrow().shakes.is_empty() {
		return Ok(());
	}
	let now = now(vm)?;
	let mut machine = m.borrow_mut();
	let mut shakes = std::mem::take(&mut machine.shakes);
	shakes.retain_mut(|s| {
		if !s.backing {
			let t = (now - s.start) / s.out;
			if t >= 1.0 {
				s.backing = true;
			} else {
				apply(&mut machine, vm, s, t);
			}
		}
		if s.backing {
			let t = (now - (s.start + s.out)) / s.back;
			if t >= 1.0 {
				end(&mut machine, vm, s);
				return false;
			}
			apply(&mut machine, vm, s, 1.0 - t);
		}
		true
	});
	machine.shakes = shakes;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::lerp_color;

	#[test]
	fn colours_step_channel_by_channel() {
		assert_eq!(lerp_color(0xff00_0000, 0xff00_0000, 0.3), 0xff00_0000);
		assert_eq!(lerp_color(0xff00_0000, 0xffff_ffff, 0.5), 0xff7f_7f7f);
		assert_eq!(lerp_color(0x0000_0000, 0xff60_0a0a, 1.0), 0xff60_0a0a);
	}
}
