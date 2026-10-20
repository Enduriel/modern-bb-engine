//! Cursor pictures as the system's cursor, pixel for pixel at any display scale (see
//! `game_cursor`).
//!
//! winit's custom cursors are in physical pixels on X11, Wayland and Windows. On macOS
//! they are in points, so a scaled (Retina) display would show them larger; there the
//! cursor is built through AppKit instead ([`mac`]): an image of the picture's pixels
//! whose size in points is pixels / scale.

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
		}
		window.set_cursor_visible(cursor.is_some());
	}
}
