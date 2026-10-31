//! Where a positional sound sits between the ears: by its place on the screen.
//!
//! A deliberate divergence from the original. Its listener is 1 unit off the sounds'
//! plane (see [`gains`](crate::gains)), so every sound left of the camera pans alike
//! (left 0.87, right 0.50, at 5 units or at 500) and only a few units around the
//! centre pan in between. Here the pan grows with the sound's sideways offset from the
//! listener, measured against the view's half-width in world units (half the screen's
//! width times the zoom): as if the listener sat that far above the plane, for the
//! direction only. A sound at the centre plays centred, one at the screen's edge is at
//! 45°, which is the original's pan for every off-centre sound (0.87 / 0.50), and one
//! past the edge stays there. Only the horizontal offset counts: the pan says where
//! on the screen a sound is, left to right.
//!
//! Everything else is the original's: the distance rolloff (measured in the plane, as
//! [`gains::positional`](crate::gains::positional) measures it) and the equal-power
//! split between the ears.

use std::f64::consts::{FRAC_PI_2, PI};

use crate::gains::{self, Gains, Listener};

/// What the listener hears from: where it is, and half the view's width in world
/// units (0 or less: unknown).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
	pub listener: Listener,
	pub half_width: f32,
}

impl View {
	/// The library's default listener, before any scene sets one.
	pub const DEFAULT: View = View {
		listener: Listener::Default,
		half_width: 0.0,
	};
}

/// A positional sound's gains: its volume, where it is, the view, the rolloff. Before
/// a scene has set the listener (or with no width to measure against) it pans as the
/// original does.
pub fn positional(volume: f32, at: (f32, f32), view: View, rolloff: f32) -> Gains {
	let Listener::At(x, _) = view.listener else {
		return gains::positional(volume, at, view.listener, rolloff);
	};
	if !(view.half_width > 0.0) {
		return gains::positional(volume, at, view.listener, rolloff);
	}
	let gain = gains::distance_gain(volume, at, view.listener, rolloff);
	gains::split(gain, share(f64::from(at.0) - f64::from(x), f64::from(view.half_width)))
}

/// How far right (0 left, 1 right, ½ centred) a sound `dx` units right of the
/// listener is, with the screen's edge `half_width` units away: its angle from a
/// listener `half_width` above the plane, at most 45° either way.
fn share(dx: f64, half_width: f64) -> f32 {
	let angle = (dx / half_width).clamp(-1.0, 1.0).atan();
	((angle + FRAC_PI_2) / PI) as f32
}

#[cfg(test)]
mod tests {
	use super::*;

	const ROLLOFF: f32 = 0.3;

	fn close(a: Gains, b: Gains, eps: f32) -> bool {
		(a.0 - b.0).abs() < eps && (a.1 - b.1).abs() < eps
	}

	fn view(x: f32, y: f32, half_width: f32) -> View {
		View {
			listener: Listener::At(x, y),
			half_width,
		}
	}

	#[test]
	fn the_original_pans_by_side_only() {
		// The kept replica (`gains::positional`), camera at (1000, 800): about the same
		// pan 5 or 500 units off (the band's edge is near -1, so 5 units left is past it).
		let listener = Listener::At(1000.0, 800.0);
		let at = |dx: f32| gains::positional(1.0, (1000.0 + dx, 800.0), listener, ROLLOFF);
		assert!(close(at(-500.0), (0.866, 0.5), 0.01), "{:?}", at(-500.0));
		assert!(close(at(-5.0), (0.90, 0.43), 0.01), "{:?}", at(-5.0));
		assert!(close(at(1.5), (0.66, 0.75), 0.01), "{:?}", at(1.5));
		assert!(close(at(500.0), (0.5, 0.866), 0.01), "{:?}", at(500.0));
	}

	#[test]
	fn the_centre_plays_centred() {
		let (l, r) = positional(1.0, (1000.0, 800.0), view(1000.0, 800.0, 960.0), ROLLOFF);
		assert_eq!(l, r);
		assert!((l - 0.5f32.sqrt()).abs() < 1e-6, "{l}");
		// Straight above or below the camera is centred too: only x counts.
		let (l, r) = positional(1.0, (1000.0, 1200.0), view(1000.0, 800.0, 960.0), ROLLOFF);
		assert_eq!(l, r);
	}

	#[test]
	fn the_pan_grows_with_the_offset() {
		let v = view(3000.0, 2000.0, 960.0);
		let mut last = positional(1.0, (3000.0 - 2000.0, 2000.0), v, ROLLOFF);
		for dx in (-1999..=2000).map(|i| i as f32) {
			let (l, r) = positional(1.0, (3000.0 + dx, 2000.0), v, ROLLOFF);
			let (share, last_share) = (r * r / (l * l + r * r), last.1 * last.1 / (last.0 * last.0 + last.1 * last.1));
			assert!(share >= last_share - 1e-6, "{dx}: {share} after {last_share}");
			last = (l, r);
		}
		// Strictly, within the view.
		let right = |dx: f32| {
			let (l, r) = positional(1.0, (3000.0 + dx, 2000.0), v, ROLLOFF);
			r * r / (l * l + r * r)
		};
		let shares = [-900.0, -500.0, -100.0, -5.0, 0.0, 5.0, 100.0, 500.0, 900.0].map(right);
		assert!(shares.windows(2).all(|w| w[0] < w[1]), "{shares:?}");
	}

	#[test]
	fn the_edge_and_beyond_pan_as_the_originals_sides() {
		let v = view(1000.0, 800.0, 960.0);
		for dx in [960.0, 1500.0, 50_000.0] {
			let right = positional(1.0, (1000.0 + dx, 800.0), v, ROLLOFF);
			let left = positional(1.0, (1000.0 - dx, 800.0), v, ROLLOFF);
			let gain = gains::distance_gain(1.0, (1000.0 + dx, 800.0), v.listener, ROLLOFF);
			assert!(close(right, (0.5 * gain, 0.75f32.sqrt() * gain), 1e-6), "{dx}: {right:?}");
			assert!(close(left, (right.1, right.0), 1e-6), "{dx}: {left:?} vs {right:?}");
		}
		// Halfway to the edge: atan(½) = 26.6°, t = 0.648.
		let (l, r) = positional(1.0, (1480.0, 800.0), v, ROLLOFF);
		assert!(close((l, r), (0.352f32.sqrt(), 0.648f32.sqrt()), 1e-3), "{l}, {r}");
	}

	#[test]
	fn the_view_scales_with_the_zoom() {
		// Zoomed out twice as far, a sound twice as far off pans the same.
		let near = positional(1.0, (1300.0, 800.0), view(1000.0, 800.0, 960.0), 1.0);
		let far = positional(1.0, (1600.0, 800.0), view(1000.0, 800.0, 1920.0), 1.0);
		let share = |(l, r): Gains| r * r / (l * l + r * r);
		assert!((share(near) - share(far)).abs() < 1e-6, "{near:?} vs {far:?}");
	}

	#[test]
	fn the_attenuation_is_the_originals() {
		for listener in [(1000.0, 800.0), (3000.0, 2000.0), (0.0, 0.0)] {
			let v = view(listener.0, listener.1, 1200.0);
			for at in [(0.0, 0.0), (5.0, 3.0), (-500.0, 10.0), (2500.0, -1200.0), (12_000.0, 7000.0), (listener.0, listener.1 + 3000.0)] {
				let at = (listener.0 + at.0, listener.1 + at.1);
				let (ol, or) = gains::positional(0.7, at, Listener::At(listener.0, listener.1), ROLLOFF);
				let (l, r) = positional(0.7, at, v, ROLLOFF);
				let (old, new) = ((ol * ol + or * or).sqrt(), (l * l + r * r).sqrt());
				assert!((old - new).abs() < 1e-6, "{at:?} from {listener:?}: {old} vs {new}");
			}
		}
	}

	#[test]
	fn without_a_view_it_pans_as_the_original() {
		for at in [(300.0, 0.0), (-40.0, 7.0)] {
			assert_eq!(positional(1.0, at, View::DEFAULT, ROLLOFF), gains::positional(1.0, at, Listener::Default, ROLLOFF));
			let unknown = view(10.0, 10.0, 0.0);
			assert_eq!(positional(1.0, at, unknown, ROLLOFF), gains::positional(1.0, at, unknown.listener, ROLLOFF));
		}
	}
}
