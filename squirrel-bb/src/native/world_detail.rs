//! World-map details: the decorations a tile carries (`tile.spawnDetail(..)`), kept
//! on the tile for the renderer.
//!
//! A detail's data is shared between its tile and the `WorldDetail` object scripts
//! get back, of the sprite class (see `sprite_class`), so script changes reach the
//! renderer and no VM handle is stored. The scene gets each as it's drawn
//! ([`SceneDetail`]): its colour with any fade at the time the map last turned.
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
use crate::native::sprite_anim::Anim;
use crate::native::sprite_class::{self, Flag, Kind, Num, SpriteHandle, SpriteState};
use crate::native::sprite_color::SpriteColor;
use crate::scene::SceneDetail;

/// One detail on a tile.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailData {
	/// Its brush's name, shared ([`Sprites::name`](super::brushes::Sprites::name)).
	pub brush: Rc<str>,
	/// Draw layer, a `Const.World.ZLevel`: higher is further back.
	pub z_level: i64,
	/// `Const.World.DetailType` bits.
	pub flags: i64,
	pub flipped: bool,
	/// `Color` and `Alpha`, with any fade in progress.
	pub(crate) color: SpriteColor,
	/// `Visible`; `fadeOutAndHide` clears it as its fade ends.
	pub visible: bool,
	/// `Highlight` (ARGB), blended over it by `HighlightFactor`.
	pub highlight: u32,
	pub highlight_factor: f64,
	pub ignore_camera_flip: bool,
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
	pub ignore_ambient: bool,
	/// Its entity id: a detail takes one from the counter every entity shares, as the
	/// original's decals do (worldgen spawns some 18,000).
	pub id: i64,
	/// Where it is: its spawn tile's centre plus its offset, in world units.
	pub pos: (f32, f32),
	/// Its offset from the tile's centre: the spawn's, or `setOffset`'s since.
	pub offset: (f32, f32),
	/// Its draw order key (the original's Z): `trunc(y)` + the brush's depth bias + the
	/// z level − the spawn tile's axial-r parity.
	pub z: i32,
	/// The sprite flag bits its brush calls left (see [`Sprites::brush_bits`]).
	pub brush_bits: u32,
	/// On the change log, waiting for a renderer to take it (see [`Changes`]).
	pub(crate) logged: bool,
}

impl Default for DetailData {
	fn default() -> Self {
		DetailData {
			brush: Rc::from(""),
			z_level: 0,
			flags: 0,
			flipped: false,
			color: SpriteColor::default(),
			visible: true,
			highlight: 0,
			highlight_factor: 0.0,
			ignore_camera_flip: false,
			spin: Spin::default(),
			anim: Anim::default(),
			frame: 0,
			rotation: 0.0,
			saturation: 1.0,
			scale: 1.0,
			ignore_ambient: false,
			id: 0,
			pos: (0.0, 0.0),
			offset: (0.0, 0.0),
			z: 0,
			brush_bits: 0,
			logged: false,
		}
	}
}

impl DetailData {
	/// It as the scene draws it at real time `now` (seconds).
	pub(crate) fn scene(&self, now: f32) -> SceneDetail {
		let shown = self.color.shown(now);
		SceneDetail {
			id: self.id,
			hidden: !self.visible,
			brush: self.brush.clone(),
			z_level: self.z_level,
			flags: self.flags,
			flipped: self.flipped,
			alpha: i64::from(shown >> 24),
			tint: shown & 0x00ff_ffff,
			frame: self.frame,
			rotation: self.rotation,
			saturation: self.saturation,
			scale: self.scale,
			ignore_ambient: self.ignore_ambient,
			pos: self.pos,
			z: self.z,
		}
	}
}

/// A detail as shared by its tile and its script object.
pub(crate) type Detail = Rc<RefCell<DetailData>>;

/// The details whose look changed since a renderer last took them: a script set
/// something on one, or it turned, stepped or faded on its own (see [`turn_all`]). A
/// renderer that keeps the map's geometry from frame to frame redraws only these
/// (adding or removing a detail changes the map's revision instead). Each detail is on
/// it once ([`DetailData::logged`]); one gone since is passed over.
#[derive(Clone, Default)]
pub(crate) struct Changes(Rc<RefCell<ChangeLog>>);

#[derive(Default)]
struct ChangeLog {
	details: Vec<Weak<RefCell<DetailData>>>,
	/// The length at which the details gone are dropped from the log: nothing may ever
	/// take it (a headless engine), and a dropped detail's memory stays while it's here.
	prune_at: usize,
}

impl Changes {
	/// Log `detail` (`data` is it, borrowed).
	pub(crate) fn note(&self, detail: &Detail, data: &mut DetailData) {
		if std::mem::replace(&mut data.logged, true) {
			return;
		}
		let mut log = self.0.borrow_mut();
		if log.details.len() >= log.prune_at {
			log.details.retain(|d| d.strong_count() > 0);
			log.prune_at = (2 * log.details.len()).max(1024);
		}
		log.details.push(Rc::downgrade(detail));
	}

	/// Each detail logged and still on the map, as `look` reads it, and start over.
	pub(crate) fn take<T>(&self, mut look: impl FnMut(&DetailData) -> T) -> Vec<T> {
		let details = std::mem::take(&mut self.0.borrow_mut().details);
		details
			.iter()
			.filter_map(Weak::upgrade)
			.map(|d| {
				let mut d = d.borrow_mut();
				d.logged = false;
				look(&d)
			})
			.collect()
	}
}

/// The details that may turn (those given a torque), animate or fade, so a frame
/// needn't visit every detail on the map.
pub(crate) type Spinning = Rc<RefCell<Vec<Weak<RefCell<DetailData>>>>>;

/// Keep `detail` turning if it has a torque, stepping while its animation may (one not
/// started yet included: its first step drops a still brush), and drawn while it fades,
/// while it shows (see [`SpriteState::may_move`]).
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

/// Whether a frame may change `d` (see [`SpriteState::may_move`]).
fn moves(d: &DetailData) -> bool {
	d.may_move()
}

/// Turn the spinning details to virtual time `now` (seconds), step the animated ones
/// to virtual millisecond `now_ms` (those shown, see `SpriteState::turns`), draw the
/// fading ones at real time `real` (a draw finishes a fade, see `SpriteColor`, which
/// may hide the detail), log each that may look different now on `changes` (one that
/// faded is drawn at `real` from now on), and forget those gone, still or hidden
/// (showing one notes it again).
pub(crate) fn turn_all(spinning: &Spinning, sprites: &Sprites, changes: &Changes, now: f32, now_ms: u32, real: f32) {
	spinning.borrow_mut().retain(|detail| {
		let Some(detail) = detail.upgrade() else { return false };
		let mut d = detail.borrow_mut();
		let d = &mut *d;
		let before = (d.rotation, d.scale, d.frame, d.visible);
		let fading = !d.color.is_fading_done();
		if d.turns() {
			d.rotation = f64::from(d.spin.turn(d.rotation as f32, now));
			if let Some(scale) = d.anim.step_on(sprites, &d.brush, now_ms) {
				d.scale = f64::from(scale);
			}
		}
		d.frame = d.anim.frame();
		if d.visible && d.color.draw(real).hide {
			d.visible = false;
		}
		if fading || before != (d.rotation, d.scale, d.frame, d.visible) {
			changes.note(&detail, d);
		}
		moves(d)
	});
}

impl SpriteState for DetailData {
	fn color(&self) -> &SpriteColor {
		&self.color
	}
	fn color_mut(&mut self) -> &mut SpriteColor {
		&mut self.color
	}
	fn anim(&self) -> &Anim {
		&self.anim
	}
	fn anim_mut(&mut self) -> &mut Anim {
		&mut self.anim
	}
	fn spin(&self) -> &Spin {
		&self.spin
	}
	fn spin_mut(&mut self) -> &mut Spin {
		&mut self.spin
	}
	fn brush(&self) -> &str {
		&self.brush
	}
	fn store_brush(&mut self, sprites: &Sprites, brush: String) {
		self.brush_bits = sprites.brush_bits(&brush, self.brush_bits);
		self.brush = sprites.name(&brush);
	}
	// It moves with its offset; its draw order stays the spawn's.
	fn set_offset(&mut self, x: f64, y: f64) {
		let (x, y) = (x as f32, y as f32);
		self.pos = (self.pos.0 - self.offset.0 + x, self.pos.1 - self.offset.1 + y);
		self.offset = (x, y);
	}
	fn num(&self, n: Num) -> f64 {
		match n {
			Num::Scale => self.scale,
			Num::Rotation => self.rotation,
			Num::Saturation => self.saturation,
			Num::HighlightFactor => self.highlight_factor,
		}
	}
	fn set_num(&mut self, n: Num, v: f64) {
		match n {
			Num::Scale => self.scale = v,
			Num::Rotation => self.rotation = v,
			Num::Saturation => self.saturation = v,
			Num::HighlightFactor => self.highlight_factor = v,
		}
	}
	fn flag(&self, f: Flag) -> bool {
		match f {
			Flag::Visible => self.visible,
			Flag::Flipped => self.flipped,
			Flag::IgnoreAmbientColor => self.ignore_ambient,
			Flag::IgnoreCameraFlip => self.ignore_camera_flip,
		}
	}
	fn set_flag(&mut self, f: Flag, on: bool) {
		match f {
			Flag::Visible => self.visible = on,
			Flag::Flipped => self.flipped = on,
			Flag::IgnoreAmbientColor => self.ignore_ambient = on,
			Flag::IgnoreCameraFlip => self.ignore_camera_flip = on,
		}
	}
	fn highlight(&self) -> u32 {
		self.highlight
	}
	fn set_highlight(&mut self, argb: u32) {
		self.highlight = argb;
	}
}

/// The payload of a `WorldDetail` instance: the detail, and the log a change made
/// through it goes on.
#[derive(Clone)]
pub(crate) struct DetailHandle {
	pub(crate) detail: Detail,
	pub(crate) changes: Changes,
}

impl SpriteHandle for DetailHandle {
	fn with<R>(&self, f: impl FnOnce(&dyn SpriteState) -> R) -> R {
		f(&*self.detail.borrow())
	}
	fn with_mut<R>(&mut self, f: impl FnOnce(&mut dyn SpriteState) -> R) -> R {
		let mut data = self.detail.borrow_mut();
		let out = f(&mut *data);
		self.changes.note(&self.detail, &mut data);
		out
	}
}

/// Build the `WorldDetail` class returned by `tile.spawnDetail(...)`: the sprite class,
/// whose details go on `spinning` once they may change on their own.
pub(crate) fn build_class(vm: &mut SQVM, sprites: &Sprites, spinning: &Spinning) -> SqClass<DetailHandle> {
	let spinning = spinning.clone();
	let moving: Rc<dyn Fn(&DetailHandle)> = Rc::new(move |d: &DetailHandle| note_spin(&spinning, &d.detail));
	let b = ClassBuilder::<DetailHandle>::new("WorldDetail");
	sprite_class::members(b, Kind { sprites: sprites.clone(), battle: false, moving: Some(moving) })
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

/// Remove the details `clear` selects, giving `removed` each one's id.
pub(crate) fn clear(details: &mut Vec<Detail>, which: Clear, mut removed: impl FnMut(i64)) {
	details.retain(|d| {
		let d = d.borrow();
		let keep = match which {
			Clear::All => false,
			Clear::Matching(mask) => d.flags & mask == 0,
			Clear::AllBut(mask) => d.flags & mask != 0,
		};
		if !keep {
			removed(d.id);
		}
		keep
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
	fn a_fade_out_and_hide_hides_it_when_drawn_after_it_ends() {
		let (spinning, sprites, changes): (Spinning, Sprites, Changes) = Default::default();
		let d = detail(0);
		d.borrow_mut().color.fade_out(100, 1.0, true);
		note_spin(&spinning, &d);
		assert!(d.borrow().color.is_fading(1.05));
		turn_all(&spinning, &sprites, &changes, 0.0, 0, 1.05);
		assert!(d.borrow().visible, "still fading");
		assert_eq!(changes.take(|d| d.visible), [true], "its fade shows");
		turn_all(&spinning, &sprites, &changes, 0.0, 0, 1.2);
		assert_eq!(changes.take(|d| d.visible), [false], "hidden as its fade ends");
		let done = d.borrow();
		assert!(!done.visible && done.color.is_fading_done() && !done.color.is_fading(1.2));
		assert!(spinning.borrow().is_empty(), "nothing left to step");
		assert_eq!(done.scene(1.2).alpha, 0);
	}

	#[test]
	fn a_change_is_logged_once_until_taken_and_a_detail_gone_is_passed_over() {
		let changes = Changes::default();
		let (a, b) = (detail(1), detail(2));
		for d in [&a, &a, &b] {
			changes.note(d, &mut d.borrow_mut());
		}
		drop(b);
		assert_eq!(changes.take(|d| d.flags), [1]);
		assert!(changes.take(|d| d.flags).is_empty());
		changes.note(&a, &mut a.borrow_mut());
		assert_eq!(changes.take(|d| d.flags), [1], "logged again once taken");
	}

	#[test]
	fn an_offset_moves_it_from_its_tiles_centre() {
		let d = detail(0);
		{
			let mut d = d.borrow_mut();
			(d.pos, d.offset) = ((105.0, 210.0), (5.0, 10.0));
			d.set_offset(-3.0, 0.0);
		}
		assert_eq!((d.borrow().pos, d.borrow().offset), ((97.0, 200.0), (-3.0, 0.0)));
	}

	#[test]
	fn clear_selects_by_type_bits() {
		let all = || vec![detail(0), detail(1), detail(2), detail(3)];
		let mut d = all();
		clear(&mut d, Clear::All, |_| {});
		assert!(d.is_empty());
		let mut d = all();
		clear(&mut d, Clear::Matching(1), |_| {});
		assert_eq!(flags(&d), [0, 2]);
		let mut d = all();
		clear(&mut d, Clear::AllBut(1), |_| {});
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
