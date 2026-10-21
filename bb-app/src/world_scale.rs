//! The world and battles drawn at the game's screen's size and enlarged, on a Retina
//! Mac, with the UI over them at the display's pixels.
//!
//! On macOS the game's screen is in points (`screen_scale`), and the frame is drawn at
//! the display's pixels: 5120x2880 for a 2560x1440 screen at 2x. Drawn there, the world
//! map took the M4's GPU 10.5-12 ms a frame (`world_pan --fit 2`, median, zoom 1 and
//! all the way out; 2.3 at 2560x1440): with the page, the compositor and a 60 Hz
//! display's 16.7 ms, a window showed ~47 frames a second. Its pictures are
//! made for a pixel of theirs to a pixel of the screen, so drawn at the screen's size
//! and each pixel copied 2x2 they look as on a display of the screen's size without
//! Retina, at a quarter of the GPU's work. The UI page (its text) and the cursor stay at
//! the display's pixels.
//!
//! `render.retina_world = true` in `settings.cfg` draws the world at the display's
//! pixels instead. Elsewhere the screen is in pixels and the frame its size: nothing to
//! enlarge.

use bb_render::{Gpu, Presenter, RenderTarget, TARGET_FORMAT};

/// The whole factor a frame of `frame` pixels is of a screen of `screen`, if it's 2 or
/// more each way: the world is then drawn at the screen's size and enlarged by it.
pub fn factor(frame: (u32, u32), screen: (u32, u32)) -> Option<u32> {
	let (sw, sh) = (screen.0.max(1), screen.1.max(1));
	let n = frame.0 / sw;
	(n >= 2 && frame == (sw * n, sh * n)).then_some(n)
}

/// What draws the world at the screen's size and enlarges it into the frame.
pub struct WorldScale {
	/// Whether the world is drawn at the screen's size where the frame is a whole
	/// multiple of it (`render.retina_world` unset).
	pub on: bool,
	target: Option<RenderTarget>,
	enlarger: Option<Presenter>,
}

impl WorldScale {
	pub fn new(on: bool) -> WorldScale {
		WorldScale {
			on,
			target: None,
			enlarger: None,
		}
	}

	/// The factor the world is enlarged by into a frame of `frame` pixels for a screen
	/// of `screen` (see [`factor`]), if it's drawn at the screen's size; the target for
	/// it is kept only while it is.
	pub fn factor(&mut self, frame: (u32, u32), screen: (u32, u32)) -> Option<u32> {
		let n = factor(frame, screen).filter(|_| self.on);
		if n.is_none() {
			self.target = None;
		}
		n
	}

	/// The target of `size` to draw the world into.
	pub fn target(&mut self, gpu: &Gpu, size: (u32, u32)) -> &RenderTarget {
		if self.target.as_ref().is_none_or(|t| (t.width, t.height) != size) {
			self.target = Some(RenderTarget::new(gpu, size.0, size.1));
		}
		self.target.as_ref().expect("made above")
	}

	/// Copy the world drawn (by [`target`](Self::target)) into all of `frame`, each of
	/// its pixels the same number of the frame's each way (nearest texels).
	pub fn enlarge(&mut self, gpu: &Gpu, frame: &RenderTarget) {
		let Some(world) = &self.target else { return };
		let enlarger = self.enlarger.get_or_insert_with(|| Presenter::new(gpu, TARGET_FORMAT));
		enlarger.draw(gpu, world, &frame.view, (0, 0, frame.width, frame.height));
	}

	/// The bytes its target takes on the GPU.
	pub fn bytes(&self) -> u64 {
		self.target.as_ref().map_or(0, RenderTarget::bytes)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn whole_factors_of_two_and_more() {
		assert_eq!(factor((5120, 2880), (2560, 1440)), Some(2), "a 2x Retina display");
		assert_eq!(factor((5120, 2576), (2560, 1288)), Some(2), "a window on it");
		assert_eq!(factor((2560, 1440), (2560, 1440)), None, "pixels: nothing to enlarge");
		assert_eq!(factor((5120, 2880), (1920, 1080)), None, "a mode scaled to the display");
		assert_eq!(factor((5121, 2880), (2560, 1440)), None);
		assert_eq!(factor((7680, 4320), (2560, 1440)), Some(3));
	}
}
