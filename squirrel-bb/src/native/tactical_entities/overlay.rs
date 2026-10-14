//! A unit's bars over its head (`createOverlay`), as the original's overlay keeps them
//! (`0x4b3250`, `setOverlayValues` `0x4b3c70`, `setOverlayIcons` `0x4b42f0`;
//! disassembled 2026-10-14):
//! - three bars, head armour, body armour and hitpoints, each its value in 24ths,
//!   rounded (`entityoverlay_bar_N`; none at 0);
//! - under each bar a lighter twin: when a value drops, it shows what the bar showed
//!   before and fades out over a second of real time (`#d0d3e5` for armour,
//!   `#ffadad` for hitpoints); a rise hides it;
//! - the status icons, brush names in order.
//!
//! With the battle's stats toggle (`ShowOverlayStats`, temporary) on, the overlay
//! always shows. Off, it shows for 1.35 s of real time after a bar changes, fading in
//! and out over 125 ms (`0x4b39d0`, `0x4c3520`); the lost-value twins keep their own
//! fade. That flash is gated by gameplay setting byte 3 (`0x65f857`; the gameplay
//! bytes start at `0x65f854`, `AdjustCameraLevel` being byte 1), which no script can
//! read or set: taken as on.

use crate::native::sprite_color::SpriteColor;
use crate::scene::tactical::SceneOverlay;

/// The lost-value twins' colours: head armour, body armour, hitpoints.
const LOST: [u32; 3] = [0xffd0_d3e5, 0xffd0_d3e5, 0xffff_adad];

/// How long a lost-value twin fades, in milliseconds.
const LOST_FADE_MS: i64 = 1000;

/// How long the overlay shows after a change with the stats toggle off, in seconds.
const SHOWN_AFTER_CHANGE: f32 = 1.35;

/// How long it fades in and out then, in seconds.
const SHOW_FADE: f32 = 0.125;

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Overlay {
	/// Each bar's value in 24ths.
	steps: [i32; 3],
	/// Each bar's twin: what the bar showed before its last drop, and its fading colour.
	lost: [Option<(i32, SpriteColor)>; 3],
	pub icons: Vec<String>,
	/// Real time it shows until after its last change.
	until: f32,
	/// Real time it last started showing after a change.
	shown_at: f32,
}

/// Which settings show the overlay: the battle's toggle, and the hidden setting that
/// flashes it after a change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shown {
	pub always: bool,
	pub after_change: bool,
}

/// A value in 24ths, rounded half away from zero.
fn to_steps(value: f32) -> i32 {
	let v = value * 24.0;
	(if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i32
}

impl Overlay {
	/// `setOverlayValues(head, body, hitpoints)` at real time `now` (seconds).
	pub fn set_values(&mut self, values: [f32; 3], now: f32) {
		for (i, value) in values.into_iter().enumerate() {
			let (steps, before) = (to_steps(value), self.steps[i]);
			if steps == before {
				continue;
			}
			// Hidden or fading out, it fades in again.
			if now >= self.until - SHOW_FADE {
				self.shown_at = now;
			}
			self.until = now + SHOWN_AFTER_CHANGE;
			self.lost[i] = (before > 0 && steps < before).then(|| {
				let mut color = SpriteColor::new(LOST[i]);
				color.fade_out(LOST_FADE_MS, now, true);
				(before, color)
			});
			self.steps[i] = steps;
		}
	}

	/// How it shows at real time `now`, if at all.
	pub fn scene(&self, now: f32, shown: Shown) -> Option<SceneOverlay> {
		let alpha = if shown.always {
			1.0
		} else if shown.after_change && now < self.until {
			let left = self.until - now;
			if left <= SHOW_FADE { left / SHOW_FADE } else { ((now - self.shown_at) / SHOW_FADE).min(1.0) }
		} else {
			return None;
		};
		Some(SceneOverlay {
			bars: self.steps,
			lost: self.lost.map(|lost| lost.map(|(steps, color)| (steps, color.shown(now)))),
			icons: self.icons.clone(),
			alpha,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const ALWAYS: Shown = Shown { always: true, after_change: true };

	#[test]
	fn values_show_in_24ths() {
		let mut o = Overlay::default();
		o.set_values([1.0, 0.5, 0.01], 0.0);
		let s = o.scene(0.0, ALWAYS).unwrap();
		assert_eq!(s.bars, [24, 12, 0]);
		assert_eq!(s.lost, [None; 3], "nothing was lost");
	}

	#[test]
	fn with_the_toggle_off_it_shows_briefly_after_a_change() {
		let after = Shown { always: false, after_change: true };
		let mut o = Overlay::default();
		o.set_values([1.0, 1.0, 1.0], 10.0);
		let alpha = |o: &Overlay, t: f32| o.scene(t, after).map(|s| s.alpha);
		assert_eq!(alpha(&o, 10.0625), Some(0.5), "fading in");
		assert_eq!(alpha(&o, 11.0), Some(1.0));
		assert!((alpha(&o, 11.3).unwrap() - 0.4).abs() < 1e-4, "fading out");
		assert_eq!(alpha(&o, 11.35), None);
		o.set_values([1.0, 1.0, 0.5], 11.0);
		assert_eq!(o.scene(11.0, after).map(|s| s.alpha), Some(1.0), "still showing: no new fade");
		assert_eq!(o.scene(11.0, Shown { always: false, after_change: false }), None);
	}

	#[test]
	fn a_drop_leaves_a_fading_twin_and_a_rise_hides_it() {
		let mut o = Overlay::default();
		o.set_values([1.0, 1.0, 1.0], 0.0);
		o.set_values([1.0, 1.0, 0.5], 10.0);
		let s = o.scene(10.5, ALWAYS).unwrap();
		assert_eq!(s.bars, [24, 24, 12]);
		assert_eq!(s.lost[..2], [None, None]);
		let (steps, color) = s.lost[2].expect("the hitpoints' twin");
		assert_eq!(steps, 24, "what the bar showed before");
		assert_eq!(color & 0x00ff_ffff, 0x00ff_adad);
		assert_eq!(color >> 24, 127, "halfway faded");
		assert_eq!(o.scene(11.0, ALWAYS).unwrap().lost[2].map(|(_, c)| c >> 24), Some(0));
		o.set_values([1.0, 1.0, 0.75], 12.0);
		assert_eq!(o.scene(12.0, ALWAYS).unwrap().lost[2], None);
	}
}
