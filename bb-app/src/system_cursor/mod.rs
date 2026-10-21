//! Cursor pictures as the system's cursor, a pixel of the picture to a pixel of the
//! game's screen (see `game_cursor` and `screen_scale`).
//!
//! winit's custom cursors are in physical pixels on X11, Wayland and Windows, where the
//! game's screen is too: the picture as it is. On macOS the game's screen is in points:
//! the picture is enlarged by the display's backing factor, each pixel copied (so it
//! stays sharp), and built through AppKit ([`mac`]) as an image whose size in points is
//! its pixels / scale. winit's own would declare the picture's size in points equal to
//! its pixels, which AppKit then scales smoothly.

#[cfg(target_os = "macos")]
mod mac;

use bb_assets::CursorImage;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// A picture made into a cursor for a display scale.
#[derive(Clone)]
pub struct Cursor(Inner);

#[cfg(not(target_os = "macos"))]
type Inner = winit::window::CustomCursor;
#[cfg(target_os = "macos")]
type Inner = mac::Cursor;

/// Make `image` a cursor for a display of `scale` (the window's scale factor).
pub fn make(event_loop: &ActiveEventLoop, image: &CursorImage, scale: f64) -> Result<Cursor, String> {
	let enlarged = enlarged(image, crate::screen_scale::picture_factor(crate::screen_scale::of(scale)));
	let image = &enlarged;
	#[cfg(not(target_os = "macos"))]
	{
		let _ = scale;
		let source = winit::window::CustomCursor::from_rgba(
			image.rgba.clone(),
			image.width as u16,
			image.height as u16,
			image.hotspot.0,
			image.hotspot.1,
		)
		.map_err(|e| e.to_string())?;
		Ok(Cursor(event_loop.create_custom_cursor(source)))
	}
	#[cfg(target_os = "macos")]
	{
		let _ = event_loop;
		mac::cursor(image, scale).map(Cursor)
	}
}

/// `image` with each pixel made `factor` × `factor` pixels, and its hotspot moved with
/// them.
pub fn enlarged(image: &CursorImage, factor: u32) -> CursorImage {
	if factor <= 1 {
		return image.clone();
	}
	let (width, height) = (image.width as usize, image.height as usize);
	let n = factor as usize;
	let mut rgba = Vec::with_capacity(image.rgba.len() * n * n);
	for row in image.rgba.chunks_exact(width * 4).take(height) {
		let mut wide = Vec::with_capacity(row.len() * n);
		for pixel in row.chunks_exact(4) {
			for _ in 0..n {
				wide.extend_from_slice(pixel);
			}
		}
		for _ in 0..n {
			rgba.extend_from_slice(&wide);
		}
	}
	let at = |h: u16| (u32::from(h) * factor).min(u32::from(u16::MAX)) as u16;
	CursorImage {
		width: image.width * factor,
		height: image.height * factor,
		hotspot: (at(image.hotspot.0), at(image.hotspot.1)),
		rgba,
	}
}

/// Shows cursors over one window.
pub struct Shower {
	#[cfg(target_os = "macos")]
	overlay: Option<mac::Overlay>,
}

impl Shower {
	pub fn new(window: &Window) -> Shower {
		#[cfg(not(target_os = "macos"))]
		{
			let _ = window;
			Shower {}
		}
		#[cfg(target_os = "macos")]
		{
			let overlay = mac::Overlay::new(window);
			if let Err(e) = &overlay {
				eprintln!("the system's cursor: {e}");
			}
			Shower { overlay: overlay.ok() }
		}
	}

	/// Show `cursor` over the window, or no cursor (`None`).
	pub fn show(&mut self, window: &Window, cursor: Option<&Cursor>) {
		#[cfg(not(target_os = "macos"))]
		if let Some(cursor) = cursor {
			window.set_cursor(cursor.0.clone());
		}
		#[cfg(target_os = "macos")]
		if let Some(overlay) = &self.overlay {
			overlay.show(cursor.map(|c| &c.0));
			return;
		}
		window.set_cursor_visible(cursor.is_some());
	}
}

/// The cursors now, as the system has them, for a log (macOS; empty elsewhere).
pub fn describe(shown: Option<&Cursor>) -> String {
	#[cfg(target_os = "macos")]
	return mac::describe(shown.map(|c| &c.0));
	#[cfg(not(target_os = "macos"))]
	{
		let _ = shown;
		String::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_enlarged_picture_copies_each_pixel() {
		let image = CursorImage {
			width: 2,
			height: 1,
			hotspot: (1, 0),
			rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
		};
		let big = enlarged(&image, 2);
		assert_eq!((big.width, big.height, big.hotspot), (4, 2, (2, 0)));
		let row = [1, 2, 3, 4, 1, 2, 3, 4, 5, 6, 7, 8, 5, 6, 7, 8];
		assert_eq!(big.rgba, [row, row].concat());
		assert_eq!(enlarged(&image, 1), image);
	}
}
