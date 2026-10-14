//! The phases of the icon and attack effects (`0x4bbec0`, `0x4b8af0`): a fade in, a
//! stay and a fade out, timed in virtual milliseconds from the effect's start.
//!
//! The sprite starts opaque, so the fade in shows nothing (the original fades it to
//! the alpha it already has); only the fade out is seen, from opaque to clear. The
//! original runs that fade on real time; here it runs on virtual time, which is the
//! same while the game runs at normal speed.

/// An effect's phase lengths, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Phases {
	pub fade_in: f32,
	pub stay: f32,
	pub fade_out: f32,
}

impl Phases {
	/// The alpha (0 to 1) `elapsed` ms after the start, or `None` before it or once
	/// it's over.
	pub fn alpha(&self, elapsed: f32) -> Option<f32> {
		if elapsed < 0.0 {
			return None;
		}
		let fading_from = self.fade_in + self.stay;
		if elapsed < fading_from {
			return Some(1.0);
		}
		let into = elapsed - fading_from;
		(into < self.fade_out).then(|| 1.0 - into / self.fade_out)
	}

	/// Whether it's over `elapsed` ms after the start.
	pub fn over(&self, elapsed: f32) -> bool {
		elapsed >= self.fade_in + self.stay + self.fade_out
	}
}

#[cfg(test)]
mod tests {
	use super::Phases;

	#[test]
	fn opaque_until_the_fade_out() {
		let p = Phases {
			fade_in: 100.0,
			stay: 550.0,
			fade_out: 180.0,
		};
		assert_eq!(p.alpha(-1.0), None);
		assert_eq!(p.alpha(0.0), Some(1.0), "the fade in shows nothing");
		assert_eq!(p.alpha(649.0), Some(1.0));
		assert_eq!(p.alpha(740.0), Some(0.5));
		assert_eq!(p.alpha(830.0), None);
		assert!(p.over(830.0) && !p.over(829.0));
	}
}
