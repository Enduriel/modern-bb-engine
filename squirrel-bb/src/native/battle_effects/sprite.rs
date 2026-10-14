//! A sprite effect (`spawnSpriteEffect`: a thrown net, a skull; the original's
//! `0x4b99e0`, its update `0x4b9de0`; disassembled 2026-10-14). With the script's
//! `(hold, fadeIn, fadeOut)` in ms: it fades in from clear over `fadeIn` (if any),
//! holds for `hold`, then fades out over `fadeOut` and is gone (at once if that's 0).
//! Its scale runs from the start scale to the end one over the whole time.

/// A sprite effect.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpriteEffect {
	pub brush: String,
	pub pos: (f32, f32),
	/// ARGB, its alpha replaced as it fades.
	pub color: u32,
	scale: (f32, f32),
	hold: f32,
	fade_in: f32,
	fade_out: f32,
	/// Virtual seconds it started at.
	start: f32,
}

impl SpriteEffect {
	pub fn new(brush: String, pos: (f32, f32), color: u32, scale: (f32, f32), hold: f32, fade_in: f32, fade_out: f32, now: f32) -> SpriteEffect {
		SpriteEffect {
			brush,
			pos,
			color,
			scale,
			hold,
			fade_in,
			fade_out,
			start: now,
		}
	}

	fn elapsed_ms(&self, now: f32) -> f32 {
		(now - self.start) * 1000.0
	}

	/// Its alpha (0 to 1) at virtual time `now`, `None` once it's gone.
	pub fn alpha(&self, now: f32) -> Option<f32> {
		let t = self.elapsed_ms(now);
		if t < self.fade_in {
			return Some(t / self.fade_in);
		}
		let fading = t - self.fade_in - self.hold;
		if fading < 0.0 {
			return Some(1.0);
		}
		let left = 1.0 - fading / self.fade_out;
		(left > 0.0).then_some(left)
	}

	/// Its scale at virtual time `now`.
	pub fn scale(&self, now: f32) -> f32 {
		let total = (self.hold + self.fade_in + self.fade_out) * 0.001;
		if total <= 0.0 {
			return self.scale.0;
		}
		self.scale.0 + (self.scale.1 - self.scale.0) * (now - self.start) / total
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fades_in_holds_and_fades_out() {
		// The skull: no hold, 400 ms in, 300 ms out, shrinking to a quarter.
		let skull = SpriteEffect::new("skull".into(), (0.0, 0.0), 0xffff_ffff, (1.0, 0.25), 0.0, 400.0, 300.0, 1.0);
		let near = |a: Option<f32>, b: f32| a.is_some_and(|a| (a - b).abs() < 1e-4);
		assert!(near(skull.alpha(1.2), 0.5), "fading in");
		assert!(near(skull.alpha(1.55), 0.5), "fading out");
		assert_eq!(skull.alpha(1.7), None);
		assert!((skull.scale(1.35) - 0.625).abs() < 1e-5);
		// The net: gone as its hold ends, with no fade out.
		let net = SpriteEffect::new("net".into(), (0.0, 0.0), 0xffff_ffff, (1.0, 1.0), 100.0, 100.0, 0.0, 0.0);
		assert_eq!(net.alpha(0.15), Some(1.0));
		assert_eq!(net.alpha(0.2), None);
	}
}
