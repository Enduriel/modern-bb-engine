//! The sprite class: what scripts get from a sprite slot (`getSprite`, `addSprite`), a
//! detail (`tile.spawnDetail`) or a head effect (`Tactical.spawnHeadEffect`), in the
//! world and in battles. The original has one class for them all (probed 2026-10-14 by
//! `campaign/natives_objects`: each answers the same natives, `__getTable` and
//! `__setTable`). Ours keeps each kind's data where its scene reads it, so each kind
//! implements [`SpriteState`] and takes the class's members from [`members`]:
//! - read and write: `Alpha`, `Color`, `Highlight`, `HighlightFactor`, `Scale`,
//!   `Saturation`, `Rotation`, `Torque`, `Visible`, `IgnoreAmbientColor`,
//!   `IgnoreCameraFlip`;
//! - read only (writing one raises "the index '…' does not exist"; the sprite's flags,
//!   `+0x98`; bound at `0x507555`…): `HasBrush` (a brush is set), `Finished` (see [`Anim::finished`]), `Fading` (see
//!   [`SpriteColor::is_fading`]) and `IsFadingDone` (no fade is left: a draw after one
//!   ends clears it, `0x504a80`);
//! - `setBrush`, `resetBrush`, `getBrush`, `setOffset`, `setHorizontalFlipping`,
//!   `isFlippedHorizontally`, `play`, `stop`, `finish`, `fade`, `fadeIn`, `fadeOut`,
//!   `fadeOutAndHide`, `fadeToAlpha`, `setBrightness`, `varyColor`, `varyBrightness`,
//!   `varySaturation`.
//!
//! An unknown brush clears the sprite (see `brushes`). Fades run on real time and finish
//! when the sprite is drawn (see `sprite_color`), frames run on virtual time (see
//! `sprite_anim`), and `vary…` draw from the C runtime's `rand()` and change the colour
//! once.

use std::rc::Rc;

use squirrel::{ClassBuilder, Retained, SQVM, SqStrongRef, Variadic};

use super::color::{color_argb, color_object};
use crate::native::Sprites;
use crate::native::spin::Spin;
use crate::native::sprite_anim::{Anim, virtual_ms};
use crate::native::sprite_color::{self, SpriteColor, real_time};

/// A sprite's numbers, which each kind stores as it likes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Num {
	Scale,
	/// Degrees, counter-clockwise.
	Rotation,
	Saturation,
	/// How much of `Highlight` shows over it.
	HighlightFactor,
}

/// A sprite's switches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Flag {
	Visible,
	/// Mirrored left to right (`setHorizontalFlipping`).
	Flipped,
	IgnoreAmbientColor,
	IgnoreCameraFlip,
}

/// What the class's members read and write on one kind of sprite.
pub(crate) trait SpriteState {
	fn color(&self) -> &SpriteColor;
	fn color_mut(&mut self) -> &mut SpriteColor;
	fn anim(&self) -> &Anim;
	fn anim_mut(&mut self) -> &mut Anim;
	fn spin(&self) -> &Spin;
	fn spin_mut(&mut self) -> &mut Spin;
	fn brush(&self) -> &str;
	/// Show `brush` (a known brush, or `""` for none), its frames and turn already set
	/// up for it.
	fn store_brush(&mut self, sprites: &Sprites, brush: String);
	/// `setOffset`: from where it's anchored, in world units with y up.
	fn set_offset(&mut self, x: f64, y: f64);
	fn num(&self, n: Num) -> f64;
	fn set_num(&mut self, n: Num, v: f64);
	fn flag(&self, f: Flag) -> bool;
	fn set_flag(&mut self, f: Flag, on: bool);
	/// `Highlight`, ARGB.
	fn highlight(&self) -> u32;
	fn set_highlight(&mut self, argb: u32);

	/// Whether the frame's turn steps it: the original updates a sprite as it draws one
	/// that's visible and has a brush (the callers of `0x52c120` check its brush and
	/// flag bit 0, `Visible`), so a hidden sprite's turn and frames wait, and catch up
	/// once it shows (see `spin`, `sprite_anim`).
	fn turns(&self) -> bool {
		self.flag(Flag::Visible) && !self.brush().is_empty()
	}

	/// Whether a frame's turn may change it: it [`turns`](Self::turns), with a torque
	/// or frames still to play (or its brush's not started yet).
	fn may_turn(&self) -> bool {
		self.turns() && (self.spin().torque != 0.0 || self.anim().running())
	}

	/// Whether a frame may change it on its own (see `moving_sprites`): it may turn, or
	/// it's shown with a fade (a draw ends it; a hidden sprite isn't drawn, and showing
	/// it again counts as a change).
	fn may_move(&self) -> bool {
		self.may_turn() || (self.flag(Flag::Visible) && !self.color().is_fading_done())
	}
}

/// A class's payload: a sprite, or a handle to one.
pub(crate) trait SpriteHandle: 'static {
	fn with<R>(&self, f: impl FnOnce(&dyn SpriteState) -> R) -> R;
	fn with_mut<R>(&mut self, f: impl FnOnce(&mut dyn SpriteState) -> R) -> R;
}

/// One kind's place in the class: its brushes, whether its sprites are a battle's (their
/// atlases go in the battle queue, see `brushes`), and what it does once a sprite may
/// change on its own (a brush set, `play`, a torque, a fade or shown: each kind keeps a
/// list of those to step, see `moving_sprites`).
pub(crate) struct Kind<H> {
	pub sprites: Sprites,
	pub battle: bool,
	pub moving: Option<Rc<dyn Fn(&H)>>,
}

/// Add the sprite class's members to `b`, for sprites of `kind`.
pub(crate) fn members<H: SpriteHandle>(b: ClassBuilder<H>, kind: Kind<H>) -> ClassBuilder<H> {
	let Kind { sprites, battle, moving } = kind;
	let moved: Rc<dyn Fn(&H)> = Rc::new(move |h: &H| {
		if let Some(m) = &moving {
			m(h);
		}
	});
	let crt = sprites.crt();
	let (crt_color, crt_brightness, crt_saturation) = (crt.clone(), crt.clone(), crt);
	let (set_with, get_with, reset_with) = (sprites.clone(), sprites.clone(), sprites);
	let (moved_torque, moved_brush, moved_play, moved_shown) = (moved.clone(), moved.clone(), moved.clone(), moved.clone());
	let (moved_fade, moved_in, moved_out, moved_hide, moved_alpha) =
		(moved.clone(), moved.clone(), moved.clone(), moved.clone(), moved);
	let b = numbers(flags(b, moved_shown));
	b.property(
		"Alpha",
		|h: &H| h.with(|s| s.color().alpha()),
		|h: &mut H, v: i64| h.with_mut(|s| s.color_mut().set_alpha(v)),
	)
	.property_obj_rw(
		"Color",
		|h: &H, vm: &SQVM| color_object(vm, h.with(|s| s.color().color)),
		|h: &mut H, vm: &SQVM, v: SqStrongRef| {
			let c = color_argb(vm, &v).ok_or("Color: not a color")?;
			h.with_mut(|s| s.color_mut().color = c);
			Ok(())
		},
	)
	.property_obj_rw(
		"Highlight",
		|h: &H, vm: &SQVM| color_object(vm, h.with(|s| s.highlight())),
		|h: &mut H, vm: &SQVM, v: SqStrongRef| {
			let c = color_argb(vm, &v).ok_or("Highlight: not a color")?;
			h.with_mut(|s| s.set_highlight(c));
			Ok(())
		},
	)
	.property(
		"Torque",
		|h: &H| f64::from(h.with(|s| s.spin().torque)),
		move |h: &mut H, v: f64| {
			h.with_mut(|s| s.spin_mut().torque = v as f32);
			moved_torque(h);
		},
	)
	// Read only: writing one raises "the index '…' does not exist", as the original's.
	.property_ro("HasBrush", |h: &H| h.with(|s| !s.brush().is_empty()))
	.property_ro("Finished", |h: &H| h.with(|s| s.anim().finished()))
	.property_obj("Fading", |h: &H, vm: &SQVM| SqStrongRef::bool(vm, h.with(|s| s.color().is_fading(real_time(vm)))))
	.property_ro("IsFadingDone", |h: &H| h.with(|s| s.color().is_fading_done()))
	.method("setBrush", move |h: &mut H, vm: &SQVM, brush: &str| {
		let brush = if battle { set_with.resolve_battle(brush) } else { set_with.resolve(brush) };
		let now = virtual_ms(vm);
		h.with_mut(|s| {
			let old = s.brush().to_string();
			s.spin_mut().rebrush(&set_with, &old, &brush);
			s.anim_mut().rebrush(&set_with, &old, &brush, now);
			s.store_brush(&set_with, brush);
		});
		moved_brush(h);
	})
	.method("resetBrush", move |h: &mut H| h.with_mut(|s| s.store_brush(&reset_with, String::new())))
	// `null` without a brush, else `{ Name, TextureName }`: scripts read
	// `getBrush().Name + "_top"` and hand `TextureName` to `Tactical.addResource`.
	.method("getBrush", move |h: &mut H, vm: &SQVM| -> Option<SqStrongRef> {
		h.with(|s| get_with.brush_object(vm, s.brush()))
	})
	.method("setOffset", |h: &mut H, vm: &SQVM, offset: Retained| -> Result<(), String> {
		let (x, y) = crate::native::vec::position(vm, &offset.0, "setOffset")?;
		h.with_mut(|s| s.set_offset(x, y));
		Ok(())
	})
	.method("setHorizontalFlipping", |h: &mut H, on: bool| h.with_mut(|s| s.set_flag(Flag::Flipped, on)))
	.method("isFlippedHorizontally", |h: &mut H| h.with(|s| s.flag(Flag::Flipped)))
	// Its brush's frames (see `sprite_anim`).
	.method("play", move |h: &mut H, vm: &SQVM, _a: Variadic<Retained>| {
		h.with_mut(|s| s.anim_mut().play(virtual_ms(vm)));
		moved_play(h);
	})
	.method("stop", |h: &mut H, _a: Variadic<Retained>| h.with_mut(|s| s.anim_mut().stop()))
	.method("finish", |h: &mut H, _a: Variadic<Retained>| h.with_mut(|s| s.anim_mut().finish()))
	.method("fade", move |h: &mut H, vm: &SQVM, color: SqStrongRef, ms: i64| -> Result<(), String> {
		let target = color_argb(vm, &color).ok_or("fade: not a color")?;
		h.with_mut(|s| s.color_mut().fade_to(target, ms, real_time(vm), false));
		moved_fade(h);
		Ok(())
	})
	.method("fadeIn", move |h: &mut H, vm: &SQVM, ms: i64| {
		h.with_mut(|s| s.color_mut().fade_in(ms, real_time(vm)));
		moved_in(h);
	})
	.method("fadeOut", move |h: &mut H, vm: &SQVM, ms: i64| {
		h.with_mut(|s| s.color_mut().fade_out(ms, real_time(vm), false));
		moved_out(h);
	})
	.method("fadeOutAndHide", move |h: &mut H, vm: &SQVM, ms: i64| {
		h.with_mut(|s| s.color_mut().fade_out(ms, real_time(vm), true));
		moved_hide(h);
	})
	.method("fadeToAlpha", move |h: &mut H, vm: &SQVM, alpha: i64, ms: i64| {
		h.with_mut(|s| s.color_mut().fade_to_alpha(alpha, ms, real_time(vm)));
		moved_alpha(h);
	})
	// The colour changes once (see `sprite_color`): nothing is kept to draw with.
	.method("setBrightness", |h: &mut H, v: f64| {
		h.with_mut(|s| {
			let c = s.color().color;
			s.color_mut().color = sprite_color::brighten(c, v as f32);
		})
	})
	.method("varyColor", move |h: &mut H, r: f64, g: f64, b: f64| {
		let rand = [crt_color.rand_default(), crt_color.rand_default(), crt_color.rand_default()];
		h.with_mut(|s| {
			let c = s.color().color;
			s.color_mut().color = sprite_color::vary_color(c, [r as f32, g as f32, b as f32], rand);
		});
	})
	.method("varySaturation", move |h: &mut H, v: f64| {
		let rand = crt_saturation.rand_default();
		h.with_mut(|s| {
			let saturation = sprite_color::vary_saturation(s.num(Num::Saturation) as f32, v as f32, rand);
			s.set_num(Num::Saturation, f64::from(saturation));
		});
	})
	.method("varyBrightness", move |h: &mut H, v: f64| {
		let rand = crt_brightness.rand_default();
		h.with_mut(|s| {
			let c = s.color().color;
			s.color_mut().color = sprite_color::vary_brightness(c, v as f32, rand);
		});
	})
}

/// The numeric properties: floats, as the original's.
fn numbers<H: SpriteHandle>(b: ClassBuilder<H>) -> ClassBuilder<H> {
	[("Scale", Num::Scale), ("Rotation", Num::Rotation), ("Saturation", Num::Saturation), ("HighlightFactor", Num::HighlightFactor)]
		.into_iter()
		.fold(b, |b, (name, n)| b.property(name, move |h: &H| h.with(|s| s.num(n)), move |h: &mut H, v: f64| h.with_mut(|s| s.set_num(n, v))))
}

/// The switches scripts set as properties. A sprite shown again may have a fade to
/// finish (`shown`).
fn flags<H: SpriteHandle>(b: ClassBuilder<H>, shown: Rc<dyn Fn(&H)>) -> ClassBuilder<H> {
	let b = b.property(
		"Visible",
		|h: &H| h.with(|s| s.flag(Flag::Visible)),
		move |h: &mut H, on: bool| {
			h.with_mut(|s| s.set_flag(Flag::Visible, on));
			if on {
				shown(h);
			}
		},
	);
	[("IgnoreAmbientColor", Flag::IgnoreAmbientColor), ("IgnoreCameraFlip", Flag::IgnoreCameraFlip)]
		.into_iter()
		.fold(b, |b, (name, f)| b.property(name, move |h: &H| h.with(|s| s.flag(f)), move |h: &mut H, on: bool| h.with_mut(|s| s.set_flag(f, on))))
}
