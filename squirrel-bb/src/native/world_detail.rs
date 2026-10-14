//! World-map details: the decorations a tile carries (`tile.spawnDetail(..)`), kept
//! on the tile for the renderer.
//!
//! A detail's data is shared between its tile and the `WorldDetail` object scripts
//! get back, so script changes (`Alpha`, `Torque`, `setBrush`, …) reach the renderer
//! and no VM handle is stored.
//!
//! The arguments, from every call site in the scripts: `spawnDetail(brush)`,
//! `(brush, z, type)`, `(brush, z, type, flipped)` and longer forms with an offset.
//! `z` is a `Const.World.ZLevel` (Terrain 30 at the back to Particles -30 at the
//! front) and `type` a mask of `Const.World.DetailType` bits, which `clear` and
//! `clearAllBut` select by.
//!
//! Not yet checked against the original: the defaults of a fresh detail and how the
//! masks of `clear(mask)` / `clearAllBut(mask)` select.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use squirrel::{ClassBuilder, SQVM, SqClass};

use crate::native::Sprites;
use crate::native::spin::Spin;
use crate::native::sprite_anim::{Anim, virtual_ms};

/// One detail on a tile.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailData {
	pub brush: String,
	/// Draw layer, a `Const.World.ZLevel`: higher is further back.
	pub z_level: i64,
	/// `Const.World.DetailType` bits.
	pub flags: i64,
	pub flipped: bool,
	pub alpha: i64,
	/// Its `Torque` (see `spin`).
	pub(crate) spin: Spin,
	/// Its brush's frames (see `sprite_anim`); it starts at its first step.
	pub(crate) anim: Anim,
	/// The frame of its brush it shows.
	pub frame: u32,
	/// Rotation in degrees (the original's `Rotation`, field `+0x40`).
	pub rotation: f64,
	pub saturation: f64,
	pub scale: f64,
	/// Red, green and blue (RGB), which `setBrightness` darkens; `alpha` is the rest of
	/// the original's colour.
	pub tint: u32,
	pub ignore_ambient: bool,
	/// Its entity id: a detail takes one from the counter every entity shares, as the
	/// original's decals do (worldgen spawns some 18,000).
	pub id: i64,
	/// Where it is: its spawn tile's centre plus the spawn's offset, in world units.
	pub pos: (f32, f32),
	/// Its draw order key (the original's Z): `trunc(y)` + the brush's depth bias + the
	/// z level − the spawn tile's axial-r parity.
	pub z: i32,
	/// The sprite flag bits its brush calls left (see [`Sprites::brush_bits`]).
	pub brush_bits: u32,
}

impl Default for DetailData {
	fn default() -> Self {
		DetailData {
			brush: String::new(),
			z_level: 0,
			flags: 0,
			flipped: false,
			alpha: 255,
			spin: Spin::default(),
			anim: Anim::default(),
			frame: 0,
			rotation: 0.0,
			saturation: 1.0,
			scale: 1.0,
			tint: 0x00ff_ffff,
			ignore_ambient: false,
			id: 0,
			pos: (0.0, 0.0),
			z: 0,
			brush_bits: 0,
		}
	}
}

/// A detail as shared by its tile and its script object.
pub(crate) type Detail = Rc<RefCell<DetailData>>;

/// The details that may turn (those given a torque) or animate, so a frame needn't
/// visit every detail on the map.
pub(crate) type Spinning = Rc<RefCell<Vec<Weak<RefCell<DetailData>>>>>;

/// Keep `detail` turning if it has a torque, and stepping while its animation may
/// (one not started yet included: its first step drops a still brush).
pub(crate) fn note_spin(spinning: &Spinning, detail: &Detail) {
	if !moves(&detail.borrow()) {
		return;
	}
	let mut spinning = spinning.borrow_mut();
	let weak = Rc::downgrade(detail);
	if !spinning.iter().any(|d| d.ptr_eq(&weak)) {
		spinning.push(weak);
	}
}

/// Whether a frame may change `d`: a torque, or an animation still running.
fn moves(d: &DetailData) -> bool {
	d.spin.torque != 0.0 || d.anim.running()
}

/// Turn the spinning details to virtual time `now` (seconds) and step the animated
/// ones to virtual millisecond `now_ms`, and forget those gone or still.
pub(crate) fn turn_all(spinning: &Spinning, sprites: &Sprites, now: f32, now_ms: u32) {
	spinning.borrow_mut().retain(|d| {
		let Some(d) = d.upgrade() else { return false };
		let mut d = d.borrow_mut();
		let d = &mut *d;
		d.rotation = f64::from(d.spin.turn(d.rotation as f32, now));
		if let Some(scale) = d.anim.step_on(sprites, &d.brush, now_ms) {
			d.scale = f64::from(scale);
		}
		d.frame = d.anim.frame();
		moves(d)
	});
}

/// The payload of a `WorldDetail` instance.
#[derive(Clone)]
pub(crate) struct DetailHandle(pub(crate) Detail);

impl DetailHandle {
	fn get<R>(&self, f: impl FnOnce(&DetailData) -> R) -> R {
		f(&self.0.borrow())
	}
	fn set(&self, f: impl FnOnce(&mut DetailData)) {
		f(&mut self.0.borrow_mut())
	}
}

/// Build the `WorldDetail` class returned by `tile.spawnDetail(...)`. A brush set
/// on a detail hints its atlas to `sprites`; a detail given a torque goes on
/// `spinning`.
pub(crate) fn build_class(vm: &mut SQVM, sprites: &Sprites, spinning: &Spinning) -> SqClass<DetailHandle> {
	let sprites = sprites.clone();
	let (spin_torque, spin_brush) = (spinning.clone(), spinning.clone());
	ClassBuilder::<DetailHandle>::new("WorldDetail")
		.property(
			"Alpha",
			|d: &DetailHandle| d.get(|d| d.alpha),
			|d: &mut DetailHandle, v: i64| d.set(|d| d.alpha = v),
		)
		// A float, like the original's (`0x4bcd00`).
		.property(
			"Torque",
			|d: &DetailHandle| f64::from(d.get(|d| d.spin.torque)),
			move |d: &mut DetailHandle, v: f64| {
				d.set(|d| d.spin.torque = v as f32);
				note_spin(&spin_torque, &d.0);
			},
		)
		.property(
			"Saturation",
			|d: &DetailHandle| d.get(|d| d.saturation),
			|d: &mut DetailHandle, v: f64| d.set(|d| d.saturation = v),
		)
		.property(
			"Scale",
			|d: &DetailHandle| d.get(|d| d.scale),
			|d: &mut DetailHandle, v: f64| d.set(|d| d.scale = v),
		)
		.property(
			"IgnoreAmbientColor",
			|d: &DetailHandle| d.get(|d| d.ignore_ambient),
			|d: &mut DetailHandle, v: bool| d.set(|d| d.ignore_ambient = v),
		)
		// A new brush brings its own rotation speed as the torque.
		.method("setBrush", move |d: &mut DetailHandle, vm: &SQVM, brush: &str| {
			let now = virtual_ms(vm);
			d.set(|d| {
				d.spin.rebrush(&sprites, &d.brush, brush);
				d.anim.rebrush(&sprites, &d.brush, brush, now);
				d.brush_bits = sprites.brush_bits(brush, d.brush_bits);
				d.brush = brush.to_string();
			});
			note_spin(&spin_brush, &d.0);
			sprites.hint(&sprites.texture_of(brush));
		})
		// A one-time change to its colour (see `sprite_color::brighten`).
		.method("setBrightness", |d: &mut DetailHandle, v: f64| {
			d.set(|d| {
				let c = crate::native::sprite_color::brighten(d.tint | ((d.alpha as u32 & 0xff) << 24), v as f32);
				d.tint = c & 0x00ff_ffff;
				d.alpha = i64::from(c >> 24);
			})
		})
		.method("setDirty", |_d: &mut DetailHandle, _flag: bool| {})
		.build(vm)
}

/// Which details `clear`-style calls remove.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Clear {
	/// `clear()`: every detail.
	All,
	/// `clear(mask)`: details with any of these type bits.
	Matching(i64),
	/// `clearAllBut(mask)`: details with none of these type bits.
	AllBut(i64),
}

/// Whether one of `details` has any of `mask`'s type bits (`hasDetailType`).
pub(crate) fn has_type(details: &[Detail], mask: i64) -> bool {
	details.iter().any(|d| d.borrow().flags & mask != 0)
}

/// Remove the details `clear` selects.
pub(crate) fn clear(details: &mut Vec<Detail>, which: Clear) {
	details.retain(|d| {
		let flags = d.borrow().flags;
		match which {
			Clear::All => false,
			Clear::Matching(mask) => flags & mask == 0,
			Clear::AllBut(mask) => flags & mask != 0,
		}
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	fn detail(flags: i64) -> Detail {
		Rc::new(RefCell::new(DetailData { flags, ..DetailData::default() }))
	}

	fn flags(details: &[Detail]) -> Vec<i64> {
		details.iter().map(|d| d.borrow().flags).collect()
	}

	#[test]
	fn clear_selects_by_type_bits() {
		let all = || vec![detail(0), detail(1), detail(2), detail(3)];
		let mut d = all();
		clear(&mut d, Clear::All);
		assert!(d.is_empty());
		let mut d = all();
		clear(&mut d, Clear::Matching(1));
		assert_eq!(flags(&d), [0, 2]);
		let mut d = all();
		clear(&mut d, Clear::AllBut(1));
		assert_eq!(flags(&d), [1, 3]);
	}

	#[test]
	fn has_type_matches_any_bit() {
		let d = vec![detail(0), detail(4)];
		assert!(has_type(&d, 4));
		assert!(has_type(&d, 6));
		assert!(!has_type(&d, 1));
		assert!(!has_type(&[], 4));
	}
}
