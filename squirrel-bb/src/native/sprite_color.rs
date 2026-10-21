//! [`SpriteColor`]: a sprite's colour and its fades, as the original's sprite node
//! keeps them (disassembled 2026-10-14).
//!
//! A sprite has one ARGB colour (`Color`; `Alpha` is its top byte). A fade
//! (`0x52d6a0`) records a target colour, the real time it started
//! (`Time.getRealTimeF()`) and its length, and marks the sprite fading. `fadeIn` and
//! `fadeOut` (`0x4a3b30`, `0x4a6860`) target the current colour at full or no alpha,
//! `fadeToAlpha` (`0x504b40`) at a given alpha, and `fadeOutAndHide` (`0x4a3b00`)
//! fades out and hides the sprite at the end. A zero-length fade sets the colour at
//! once.
//!
//! The fade is resolved when the sprite is drawn (`0x52cb5b`), not by an update:
//! the draw blends from the stored colour to the target by `min(elapsed / length,
//! 1)`, per channel in `f32`, truncated. The stored colour, which scripts read, keeps
//! its start value until a draw at or past the end commits the target and ends the
//! fade (`IsFadingDone`). So a sprite that isn't drawn never finishes fading.

use squirrel::{Number, SQVM, SqStrongRef, SqTable};

/// `Time.getRealTimeF()`: real seconds, the clock fades run on.
pub(crate) fn real_time(vm: &SQVM) -> f32 {
	SqTable::root(vm)
		.get::<SqStrongRef>("Time")
		.and_then(|t| t.call_method_ret::<Number, _>(vm, "getRealTimeF", ()).ok().flatten())
		.map_or(0.0, |n| n.0 as f32)
}

/// `Time.getVirtualTimeF()`: virtual seconds, the clock effects and quakes run on.
pub(crate) fn virtual_time(vm: &SQVM) -> f32 {
	SqTable::root(vm)
		.get::<SqStrongRef>("Time")
		.and_then(|t| t.call_method_ret::<Number, _>(vm, "getVirtualTimeF", ()).ok().flatten())
		.map_or(0.0, |n| n.0 as f32)
}

/// A fade in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fade {
	target: u32,
	/// Real seconds.
	start: f32,
	length: f32,
	hide: bool,
}

/// A sprite's colour, with its fade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SpriteColor {
	/// ARGB, as stored (what `Color` and `Alpha` read).
	pub color: u32,
	fade: Option<Fade>,
}

impl Default for SpriteColor {
	fn default() -> Self {
		SpriteColor::new(0xffff_ffff)
	}
}

/// What a draw found: the colour to draw with, and whether the sprite is hidden now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Drawn {
	pub color: u32,
	pub hide: bool,
}

impl SpriteColor {
	pub fn new(color: u32) -> Self {
		SpriteColor { color, fade: None }
	}

	/// `Alpha`: 0 to 255.
	pub fn alpha(&self) -> i64 {
		i64::from(self.color >> 24)
	}

	pub fn set_alpha(&mut self, alpha: i64) {
		self.color = (self.color & 0x00ff_ffff) | ((alpha as u32 & 0xff) << 24);
	}

	/// The entity-level `setAlpha(a)` (`0x4a9ac0`): any fade ends where it is, and the
	/// alpha is `a` at once.
	pub fn snap_alpha(&mut self, alpha: i64) {
		self.fade = None;
		self.set_alpha(alpha);
	}

	/// `IsFadingDone`.
	pub fn is_fading_done(&self) -> bool {
		self.fade.is_none()
	}

	/// `Fading` at real time `now`: a fade is under way and its time isn't up
	/// (`0x4b9860`). One that ran out is not fading, though not done until drawn.
	pub fn is_fading(&self, now: f32) -> bool {
		self.fade.is_some_and(|f| f.length > now - f.start)
	}

	/// Fade to `target` over `ms` milliseconds from real time `now` (seconds);
	/// `hide` hides the sprite when it ends.
	pub fn fade_to(&mut self, target: u32, ms: i64, now: f32, hide: bool) {
		if ms == 0 {
			self.color = target;
			self.fade = None;
			return;
		}
		self.fade = Some(Fade {
			target,
			start: now,
			length: ms as f32 * 0.001,
			hide,
		});
	}

	/// `fadeIn(ms)`.
	pub fn fade_in(&mut self, ms: i64, now: f32) {
		self.fade_to(self.color | 0xff00_0000, ms, now, false);
	}

	/// `fadeOut(ms)`, or `fadeOutAndHide(ms)` with `hide`.
	pub fn fade_out(&mut self, ms: i64, now: f32, hide: bool) {
		self.fade_to(self.color & 0x00ff_ffff, ms, now, hide);
	}

	/// `fadeToAlpha(alpha, ms)`.
	pub fn fade_to_alpha(&mut self, alpha: i64, ms: i64, now: f32) {
		let target = (self.color & 0x00ff_ffff) | ((alpha as u32 & 0xff) << 24);
		self.fade_to(target, ms, now, false);
	}

	/// The colour a draw at real time `now` shows, without committing the fade.
	pub fn shown(&self, now: f32) -> u32 {
		match self.fade {
			None => self.color,
			Some(f) => blend(self.color, f.target, progress(f, now)),
		}
	}

	/// A draw at real time `now`: the colour shown, committing a finished fade.
	pub fn draw(&mut self, now: f32) -> Drawn {
		let Some(f) = self.fade else {
			return Drawn {
				color: self.color,
				hide: false,
			};
		};
		let t = progress(f, now);
		let color = blend(self.color, f.target, t);
		if t >= 1.0 {
			self.color = color;
			self.fade = None;
		}
		Drawn {
			color,
			hide: t >= 1.0 && f.hide,
		}
	}
}

fn progress(f: Fade, now: f32) -> f32 {
	// Not below 0: a scene drawn at a step's time may be before the fade began (see
	// `scene::steps`).
	// (`min` first: a zero-length fade's 0 / 0 is done.)
	((now - f.start) / f.length).min(1.0).max(0.0)
}

/// `from` blended `t` of the way to `to`, per channel in `f32`, truncated.
fn blend(from: u32, to: u32, t: f32) -> u32 {
	let unit = |c: u32, shift: u32| ((c >> shift) & 0xff) as f32 * (1.0f32 / 255.0);
	[0, 8, 16, 24].iter().fold(0, |out, &shift| {
		let v = (unit(to, shift) * t + unit(from, shift) * (1.0 - t)) * 255.0;
		out | (((v as i32) as u32 & 0xff) << shift)
	})
}

/// The colour's channels in the order the original keeps them in memory (red, green,
/// blue, alpha), as shifts into our ARGB.
const CHANNELS: [u32; 4] = [16, 8, 0, 24];

/// Channel `i` (see [`CHANNELS`]) of `color` as the original reads it: `c · (1 / 255)`.
fn channel(color: u32, i: usize) -> f32 {
	((color >> CHANNELS[i]) & 0xff) as f32 * (1.0f32 / 255.0)
}

/// The channels back into a colour as the original writes them: `trunc(v · 255)`,
/// its low byte.
fn pack(v: [f32; 4]) -> u32 {
	(0..4).fold(0, |out, i| out | ((((v[i] * 255.0) as i32) as u32 & 0xff) << CHANNELS[i]))
}

/// `setBrightness(b)` (`0x5048c0`): scales the red, green and blue the sprite has now
/// by `b`, at most 1. It changes `Color` once; nothing is kept to draw with.
pub(crate) fn brighten(color: u32, b: f32) -> u32 {
	let c = |i| (channel(color, i) * b).min(1.0);
	pack([c(0), c(1), c(2), channel(color, 3)])
}

/// `varyBrightness(v)` (`0x4bc4b0`): moves red, green and blue `v` of the way to one
/// random grey, `rand` the C runtime's `rand()`, drawn once.
pub(crate) fn vary_brightness(color: u32, v: f32, rand: i64) -> u32 {
	let k = 1.0 - v;
	let grey = (rand & 0xff) as f32 * (1.0f32 / 255.0) * v;
	let c = |i| k * channel(color, i) + grey;
	pack([c(0), c(1), c(2), channel(color, 3)])
}

/// `varyColor(r, g, b)` (`0x504760`): moves each of red, green and blue its own share
/// of the way to a random level, `rand` the C runtime's three `rand()`s, in that order.
pub(crate) fn vary_color(color: u32, v: [f32; 3], rand: [i64; 3]) -> u32 {
	let c = |i: usize| (rand[i] & 0xff) as f32 * (1.0f32 / 255.0) * v[i] + (1.0 - v[i]) * channel(color, i);
	pack([c(0), c(1), c(2), channel(color, 3)])
}

/// `varySaturation(v)` (`0x504bd0`): moves the saturation `v` of the way to a random
/// one in `[0.001, 1]`, `rand` the C runtime's `rand()`.
pub(crate) fn vary_saturation(saturation: f32, v: f32, rand: i64) -> f32 {
	(1.0 - v) * saturation + ((rand % 1000) as f32 * 0.001 + 0.001) * v
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn brightness_scales_the_colour_once() {
		assert_eq!(brighten(0xff80_ff20, 0.9), 0xff73_e51c, "alpha kept");
		assert_eq!(brighten(0x8080_8080, 4.0), 0x80ff_ffff, "at most 1");
	}

	#[test]
	fn varying_moves_towards_the_random_levels() {
		// v = 0 keeps the colour; v = 1 is the random level.
		assert_eq!(vary_brightness(0xff20_4060, 0.0, 77), 0xff20_4060);
		assert_eq!(vary_brightness(0xff20_4060, 1.0, 0x1_80), 0xff80_8080, "the low byte");
		assert_eq!(vary_color(0x7f00_0000, [1.0, 0.0, 1.0], [255, 9, 16]), 0x7fff_0010, "red, green, blue");
		assert_eq!(vary_saturation(0.5, 0.0, 3), 0.5);
		assert!((vary_saturation(0.5, 1.0, 1999) - 1.0).abs() < 1e-6, "999 is the top");
	}

	#[test]
	fn a_fade_shows_progress_but_commits_on_a_finished_draw() {
		let mut c = SpriteColor::new(0x00ff_ffff);
		c.fade_in(250, 0.0);
		assert!(!c.is_fading_done());
		assert_eq!(c.shown(0.125) >> 24, 127, "halfway, truncated");
		assert_eq!(c.alpha(), 0, "scripts read the start colour meanwhile");
		assert_eq!(c.draw(0.1875).color >> 24, 191);
		assert!(!c.is_fading_done());
		// Long after its end, only a draw finishes it.
		assert_eq!(c.alpha(), 0);
		assert_eq!(c.draw(20.0), Drawn { color: 0xffff_ffff, hide: false });
		assert!(c.is_fading_done());
		assert_eq!(c.alpha(), 255);
	}

	#[test]
	fn fading_out_and_hiding_hides_on_the_finishing_draw() {
		let mut c = SpriteColor::default();
		c.fade_out(100, 0.0, true);
		assert!(!c.draw(0.05).hide);
		assert_eq!(c.draw(0.1), Drawn { color: 0x00ff_ffff, hide: true });
	}

	#[test]
	fn a_zero_length_fade_is_immediate() {
		let mut c = SpriteColor::new(0xff10_2030);
		c.fade_to_alpha(64, 0, 5.0);
		assert_eq!(c.color, 0x4010_2030);
		assert!(c.is_fading_done());
	}
}
