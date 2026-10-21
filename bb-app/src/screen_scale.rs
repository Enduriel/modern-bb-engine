//! The game's screen in the system's units: how many of the window's pixels a pixel of
//! the game's screen covers.
//!
//! The game's screen is what the scripts and the UI see: the video mode's size, the
//! window and screen size natives, the mouse's position, the Options screen's
//! resolutions, and with them the UI's layout and the cameras' zoom limits.
//!
//! - **macOS: points.** A Mac sizes everything in points, and a Retina (HiDPI) display
//!   has 2 pixels a point each way. A 5120x2880-pixel display that "looks like"
//!   2560x1440 is a 2560x1440 screen to the game, as a 2560x1440 display without Retina
//!   is: the same UI size and the same zoom range. The frame is still drawn at the
//!   display's pixels, scaled by the backing factor (the renderer's `fit`), and the page
//!   is painted at them (its device scale factor), so nothing is blurrier than the
//!   display allows.
//! - **Elsewhere: pixels**, as the original sees them on Windows and under Proton: a
//!   window's scale factor (X11's Xft.dpi, Wayland's output scale, Windows' DPI) leaves
//!   the game's screen at the monitor's pixels, and the player's UI scale is the setting
//!   that enlarges the UI.

/// Whether the game's screen is in points (macOS) rather than pixels.
pub const IN_POINTS: bool = cfg!(target_os = "macos");

/// Window pixels per pixel of the game's screen on a display of `scale_factor` (winit's
/// scale factor of the window or monitor).
pub fn of(scale_factor: f64) -> f64 {
	of_in(IN_POINTS, scale_factor)
}

fn of_in(in_points: bool, scale_factor: f64) -> f64 {
	if in_points && scale_factor.is_finite() && scale_factor > 0.0 {
		scale_factor
	} else {
		1.0
	}
}

/// `size` in window pixels as the game's screen sees it at `scale` (from [`of`]):
/// rounded to whole pixels of the screen, at least 1 each way.
pub fn to_screen(size: (u32, u32), scale: f64) -> (u32, u32) {
	let one = |n: u32| ((f64::from(n) / scale).round() as u32).max(1);
	(one(size.0), one(size.1))
}

/// The whole factor a cursor's picture is enlarged by for a screen of `scale` (from
/// [`of`]), pixel for pixel, so it stays sharp: the scale rounded, at least 1.
pub fn picture_factor(scale: f64) -> u32 {
	if scale.is_finite() { scale.round().clamp(1.0, 8.0) as u32 } else { 1 }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn points_on_macos_pixels_elsewhere() {
		assert_eq!(of_in(true, 2.0), 2.0);
		assert_eq!(of_in(true, 1.0), 1.0);
		assert_eq!(of_in(false, 2.0), 1.0, "a 2x Wayland or Windows display keeps pixels");
		assert_eq!(of_in(false, 1.25), 1.0);
		assert_eq!(of_in(true, 0.0), 1.0, "no scale reported");
		assert_eq!(of(1.0), 1.0);
	}

	#[test]
	fn window_pixels_to_the_screen() {
		assert_eq!(to_screen((5120, 2880), 2.0), (2560, 1440), "the Retina display that looks like 2560x1440");
		assert_eq!(to_screen((2560, 1440), 1.0), (2560, 1440));
		assert_eq!(to_screen((2049, 1), 2.0), (1025, 1), "rounded, never 0");
	}

	#[test]
	fn pictures_enlarge_by_whole_factors() {
		assert_eq!(picture_factor(1.0), 1);
		assert_eq!(picture_factor(2.0), 2);
		assert_eq!(picture_factor(1.25), 1);
		assert_eq!(picture_factor(f64::NAN), 1);
	}
}
