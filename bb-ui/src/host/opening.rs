//! Opening the page as soon as CEF can: on CEF's UI thread, the moment its context is
//! initialized (`OnContextInitialized`, where CEF's own clients create their browsers),
//! not when the game thread next gets to it. The game thread meanwhile opens the
//! install's archives and runs the scripts' preload, so CEF's start and the page's load
//! overlap the game's own start instead of preceding it ([`UiHost::start`](super::UiHost::start)).

use std::sync::Arc;

use cef::*;

use super::client::{self, Event, Shared};
use super::generic_family;
use super::scheme;
use crate::coui::{SCHEME, UiFiles};

/// What opening the page takes, handed to CEF's UI thread.
pub(super) struct Opening {
	pub files: Arc<UiFiles>,
	pub shared: Arc<Shared>,
	pub url: String,
	/// Whether the page's frames begin when the game says ([`PageFrames::Game`](super::PageFrames)).
	pub external_frames: bool,
	/// Chromium's frame rate for the page (`windowless_frame_rate`).
	pub frame_rate: i32,
	/// The window the page's dialogs belong to ([`HostConfig::parent_window`](super::HostConfig)).
	pub parent_window: Option<isize>,
}

impl Opening {
	/// Serve `coui://` and create the page's browser. On CEF's UI thread, once its
	/// context is initialized; [`Event::Created`] follows, or [`Event::OpenFailed`].
	pub fn open(self) {
		let shared = self.shared.clone();
		if let Err(why) = self.create() {
			shared.send(Event::OpenFailed(why));
		}
	}

	fn create(self) -> Result<(), String> {
		let mut factory = scheme::CouiFactory::new(self.files, self.shared.clone());
		if register_scheme_handler_factory(Some(&SCHEME.into()), None, Some(&mut factory)) != 1 {
			return Err("could not register the coui:// scheme".into());
		}
		let mut window_info = WindowInfo::default().set_as_windowless(parent_window(self.parent_window));
		window_info.external_begin_frame_enabled = self.external_frames.into();
		// Paints arrive as GPU buffers (`OnAcceleratedPaint`), never as pixels in memory.
		window_info.shared_texture_enabled = 1;
		// The generic font families are the original's, Windows' defaults (probed,
		// `probes/font_families.js`): `monospace` is Courier New there, where Linux's
		// default is DejaVu Sans Mono (a pixel taller a line: Modern Hooks' popup, and
		// MSU's `monospace` text). Fontconfig gives the metric-compatible fonts here.
		let browser_settings = BrowserSettings {
			windowless_frame_rate: self.frame_rate,
			background_color: 0,
			standard_font_family: generic_family("Times New Roman").as_str().into(),
			serif_font_family: generic_family("Times New Roman").as_str().into(),
			sans_serif_font_family: generic_family("Arial").as_str().into(),
			fixed_font_family: generic_family("Courier New").as_str().into(),
			// What Coherent allowed: `execCommand('copy')`/`('cut')` outside a click,
			// and `execCommand('paste')` (Plan Perks' Paste button). The text itself
			// comes from the game's clipboard (`bootstrap.js`, `crate::clipboard`).
			javascript_access_clipboard: State::ENABLED,
			javascript_dom_paste: State::ENABLED,
			..Default::default()
		};
		let mut client = client::page_client(self.shared);
		let created = browser_host_create_browser(
			Some(&window_info),
			Some(&mut client),
			Some(&self.url.as_str().into()),
			Some(&browser_settings),
			None,
			None,
		);
		if created != 1 {
			return Err("could not create the browser".into());
		}
		Ok(())
	}
}

/// The page's parent window as CEF takes it: the `HWND` on Windows; none elsewhere.
#[cfg(windows)]
fn parent_window(handle: Option<isize>) -> sys::cef_window_handle_t {
	handle.map_or(Default::default(), |hwnd| hwnd as sys::cef_window_handle_t)
}

#[cfg(not(windows))]
fn parent_window(_handle: Option<isize>) -> sys::cef_window_handle_t {
	Default::default()
}
