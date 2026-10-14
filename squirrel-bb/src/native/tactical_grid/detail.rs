//! The details on battle tiles (`tile.spawnDetail`): terrain dressing, blood, corpses,
//! icons. The original (`0x4da660` and its overloads `0x4da540`…`0x4da610`,
//! disassembled 2026-10-14):
//! - `spawnDetail(brush [, flags [, mirrored [, fadeIn [, offset]]]])`. An unknown
//!   brush spawns nothing and returns `null`. The one- and two-argument forms mirror
//!   at random, from an engine generator of their own (`0x68b318`, seeded from OS
//!   entropy, never the scripts' stream). A mirror only applies to a brush with flag
//!   `0x400`.
//! - Flag 8 first clears the tile's details carrying flag `0x10` (`0x4daba0`).
//! - A detail with the same brush and mirroring already on the tile is returned instead
//!   of a new one.
//! - A new detail sits at `offset` from the tile. A rotating brush (or one with flag
//!   `0x2000`) gets a random rotation in 0–359, and one with flag `0x1000` a random
//!   scale in 0.6–1.0, in that order. With `fadeIn` it starts transparent and fades in
//!   over 100 ms.
//! - What the script gets is the detail's sprite: its `Alpha`, `Scale`, `Saturation`,
//!   `Torque`, `Color`, `setBrightness` and `setBrush` change the detail in place.
//! - A rotating brush's speed is the detail's torque (see `spin`).

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{ClassBuilder, SQVM, SqClass, SqStrongRef};

use crate::native::spin::Spin;
use crate::native::sprite_color::{SpriteColor, real_time};
use crate::native::{EngineRng, Sprites};

/// One detail on a tile.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DetailData {
	pub brush: String,
	/// The script's flags (for `clear(flags)` and flag 8).
	pub flags: i64,
	pub mirrored: bool,
	/// From the tile's centre, in world units with y up.
	pub offset: (f32, f32),
	/// Degrees, counter-clockwise.
	pub rotation: f32,
	/// Its `Torque` (see `spin`).
	pub spin: Spin,
	pub scale: f32,
	pub saturation: f32,
	pub brightness: f32,
	pub color: SpriteColor,
}

/// A detail, shared between its tile and the handles scripts hold.
pub(crate) type Detail = Rc<RefCell<DetailData>>;

/// The payload of the `TacticalDetail` handles `spawnDetail` returns.
#[derive(Clone)]
pub(super) struct DetailHandle(pub Detail);

/// How `spawnDetail` was called.
pub(super) struct Spawn {
	pub brush: String,
	pub flags: i64,
	/// `None` for the forms that mirror at random.
	pub mirrored: Option<bool>,
	pub fade_in: bool,
	pub offset: (f32, f32),
}

/// What a tile's `spawnDetail` does to its `details`: the detail to hand back (new or
/// already there), or `None` for an unknown brush.
pub(super) fn spawn(details: &mut Vec<Detail>, spawn: Spawn, sprites: &Sprites, rng: &EngineRng, now: f32) -> Option<Detail> {
	let looks = sprites.with(|r| {
		if r.is_empty() {
			// No brushes loaded (headless tests): keep everything, plainly.
			return Some((0, 0.0));
		}
		r.get(&spawn.brush).map(|(_, s)| (s.flags, s.rotation_speed))
	});
	let (brush_flags, rotation_speed) = looks?;
	if spawn.flags & 8 != 0 {
		details.retain(|d| d.borrow().flags & 0x10 == 0);
	}
	let want_mirror = spawn.mirrored.unwrap_or_else(|| rng.range(0, 1) == 1);
	let mirrored = want_mirror && brush_flags & 0x400 != 0;
	if let Some(same) = details.iter().find(|d| {
		let d = d.borrow();
		d.brush == spawn.brush && d.mirrored == mirrored
	}) {
		return Some(same.clone());
	}
	let rotation = if rotation_speed != 0.0 || brush_flags & 0x2000 != 0 {
		rng.range(0, 359) as f32
	} else {
		0.0
	};
	let scale = if brush_flags & 0x1000 != 0 {
		rng.range(0, 40) as f32 / 100.0 + 0.6
	} else {
		1.0
	};
	let mut color = SpriteColor::default();
	if spawn.fade_in {
		color.set_alpha(0);
		color.fade_in(100, now);
	}
	let detail = Rc::new(RefCell::new(DetailData {
		brush: spawn.brush,
		flags: spawn.flags,
		mirrored,
		offset: spawn.offset,
		rotation,
		spin: Spin::new(rotation_speed),
		scale,
		saturation: 1.0,
		brightness: 1.0,
		color,
	}));
	details.push(detail.clone());
	Some(detail)
}

pub(super) fn build_class(vm: &mut SQVM, sprites: &Sprites) -> SqClass<DetailHandle> {
	let sprites = sprites.clone();
	ClassBuilder::<DetailHandle>::new("TacticalDetail")
		.property(
			"Alpha",
			|d: &DetailHandle| d.0.borrow().color.alpha(),
			|d: &mut DetailHandle, v: i64| d.0.borrow_mut().color.set_alpha(v),
		)
		.property(
			"Scale",
			|d: &DetailHandle| f64::from(d.0.borrow().scale),
			|d: &mut DetailHandle, v: f64| d.0.borrow_mut().scale = v as f32,
		)
		.property(
			"Saturation",
			|d: &DetailHandle| f64::from(d.0.borrow().saturation),
			|d: &mut DetailHandle, v: f64| d.0.borrow_mut().saturation = v as f32,
		)
		.property(
			"Torque",
			|d: &DetailHandle| f64::from(d.0.borrow().spin.torque),
			|d: &mut DetailHandle, v: f64| d.0.borrow_mut().spin.torque = v as f32,
		)
		.property(
			"IsFadingDone",
			|d: &DetailHandle| d.0.borrow().color.is_fading_done(),
			|_d: &mut DetailHandle, _v: bool| {},
		)
		.property_obj_rw(
			"Color",
			|d: &DetailHandle, vm: &SQVM| crate::native::color::color_object(vm, d.0.borrow().color.color),
			|d: &mut DetailHandle, vm: &SQVM, v: SqStrongRef| {
				d.0.borrow_mut().color.color = crate::native::color::color_argb(vm, &v).ok_or("Color: not a color")?;
				Ok(())
			},
		)
		.method("setBrightness", |d: &mut DetailHandle, v: f64| d.0.borrow_mut().brightness = v as f32)
		.method("setBrush", move |d: &mut DetailHandle, brush: String| {
			let brush = sprites.resolve(&brush);
			let mut d = d.0.borrow_mut();
			let d = &mut *d;
			d.spin.rebrush(&sprites, &d.brush, &brush);
			d.brush = brush;
		})
		.method("fadeIn", |d: &mut DetailHandle, vm: &SQVM, ms: i64| {
			d.0.borrow_mut().color.fade_in(ms, real_time(vm))
		})
		.method("fadeOut", |d: &mut DetailHandle, vm: &SQVM, ms: i64| {
			d.0.borrow_mut().color.fade_out(ms, real_time(vm), false)
		})
		.method("fadeToAlpha", |d: &mut DetailHandle, vm: &SQVM, alpha: i64, ms: i64| {
			d.0.borrow_mut().color.fade_to_alpha(alpha, ms, real_time(vm))
		})
		.build(vm)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn plain(brush: &str) -> Spawn {
		Spawn {
			brush: brush.into(),
			flags: 0,
			mirrored: Some(false),
			fade_in: false,
			offset: (0.0, 0.0),
		}
	}

	#[test]
	fn the_same_brush_and_mirroring_is_reused() {
		let (sprites, rng) = (Sprites::default(), EngineRng::seeded(1));
		let mut details = Vec::new();
		let a = spawn(&mut details, plain("blood_01"), &sprites, &rng, 0.0).unwrap();
		let b = spawn(&mut details, plain("blood_01"), &sprites, &rng, 0.0).unwrap();
		assert!(Rc::ptr_eq(&a, &b));
		spawn(&mut details, plain("blood_02"), &sprites, &rng, 0.0).unwrap();
		assert_eq!(details.len(), 2);
	}

	#[test]
	fn flag_8_clears_flag_0x10_details_first() {
		let (sprites, rng) = (Sprites::default(), EngineRng::seeded(1));
		let mut details = Vec::new();
		spawn(&mut details, Spawn { flags: 0x10, ..plain("corpse") }, &sprites, &rng, 0.0);
		spawn(&mut details, Spawn { flags: 0x2, ..plain("grass") }, &sprites, &rng, 0.0);
		spawn(&mut details, Spawn { flags: 8, ..plain("icon") }, &sprites, &rng, 0.0);
		let left: Vec<String> = details.iter().map(|d| d.borrow().brush.clone()).collect();
		assert_eq!(left, ["grass", "icon"]);
	}

	#[test]
	fn a_fading_detail_starts_transparent() {
		let (sprites, rng) = (Sprites::default(), EngineRng::seeded(1));
		let mut details = Vec::new();
		let d = spawn(&mut details, Spawn { fade_in: true, ..plain("blood") }, &sprites, &rng, 5.0).unwrap();
		let d = d.borrow();
		assert_eq!(d.color.alpha(), 0);
		assert!(!d.color.is_fading_done());
		assert_eq!(d.color.shown(6.0) >> 24, 255);
	}
}
