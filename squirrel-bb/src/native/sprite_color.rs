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

	/// `IsFadingDone`.
	pub fn is_fading_done(&self) -> bool {
		self.fade.is_none()
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
	((now - f.start) / f.length).min(1.0)
}

/// `from` blended `t` of the way to `to`, per channel in `f32`, truncated.
fn blend(from: u32, to: u32, t: f32) -> u32 {
	let unit = |c: u32, shift: u32| ((c >> shift) & 0xff) as f32 * (1.0f32 / 255.0);
	[0, 8, 16, 24].iter().fold(0, |out, &shift| {
		let v = (unit(to, shift) * t + unit(from, shift) * (1.0 - t)) * 255.0;
		out | (((v as i32) as u32 & 0xff) << shift)
	})
}

#[cfg(test)]
mod tests {
	use super::*;

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
