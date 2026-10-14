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
use std::rc::Rc;

use squirrel::{ClassBuilder, SQVM, SqClass};

use crate::native::Sprites;

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
	pub torque: i64,
	/// Rotation in degrees (the original's `Rotation`, field `+0x40`).
	pub rotation: f64,
	pub saturation: f64,
	pub scale: f64,
	pub brightness: f64,
	pub ignore_ambient: bool,
}

impl Default for DetailData {
	fn default() -> Self {
		DetailData {
			brush: String::new(),
			z_level: 0,
			flags: 0,
			flipped: false,
			alpha: 255,
			torque: 0,
			rotation: 0.0,
			saturation: 1.0,
			scale: 1.0,
			brightness: 1.0,
			ignore_ambient: false,
		}
	}
}

/// A detail as shared by its tile and its script object.
pub(crate) type Detail = Rc<RefCell<DetailData>>;

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
/// on a detail hints its atlas to `sprites`.
pub(crate) fn build_class(vm: &mut SQVM, sprites: &Sprites) -> SqClass<DetailHandle> {
	let sprites = sprites.clone();
	ClassBuilder::<DetailHandle>::new("WorldDetail")
		.property(
			"Alpha",
			|d: &DetailHandle| d.get(|d| d.alpha),
			|d: &mut DetailHandle, v: i64| d.set(|d| d.alpha = v),
		)
		.property(
			"Torque",
			|d: &DetailHandle| d.get(|d| d.torque),
			|d: &mut DetailHandle, v: i64| d.set(|d| d.torque = v),
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
		.method("setBrush", move |d: &mut DetailHandle, brush: &str| {
			d.set(|d| d.brush = brush.to_string());
			sprites.hint(&sprites.texture_of(brush));
		})
		.method("setBrightness", |d: &mut DetailHandle, v: f64| d.set(|d| d.brightness = v))
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
