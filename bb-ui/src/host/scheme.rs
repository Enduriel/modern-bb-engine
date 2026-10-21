//! The `coui://` scheme handler, on CEF's IO thread.
//!
//! - Files: served from [`UiFiles`] without involving the game thread. `coui` is
//!   registered as a standard (hierarchical, so relative URLs resolve), secure, CORS-
//!   and fetch-enabled scheme. Each first path segment is its own origin (`coui://ui`
//!   loads fonts from `coui://gfx`), so every response allows any origin, as the
//!   original's single file system did. A missing file is a network error, as in the
//!   original.
//! - The bridge isn't here: the page's blocking calls into the game travel on a socket
//!   of their own ([`bridge_wire`](super::bridge_wire)), not as requests.
//! - Procedural images (`coui://procedural/tacticalentity(...)`, `worldentity(...)`):
//!   the engine composes portraits from entity sprites. Each becomes an
//!   [`Event::Procedural`] for the game thread, which answers with a PNG; when it
//!   can't (or doesn't answer), a transparent placeholder. The original also answers
//!   200 with a fallback image when composing fails, never a load error.
//! - A PNG asked for with `?bb-upside-down` comes upside down ([`upside_down`]): the
//!   picture of a background Coherent tiles (`tiles.js`).
//! - Fonts on macOS come as their [`whole_pixel`] variants: glyphs advance by whole
//!   pixels there, as in Coherent.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use cef::*;

use super::client::{Event, Shared};
use super::responder::{Body, Responder, blank};
use crate::coui::{UiFiles, mime_type, url_to_path};
use crate::{upside_down, whole_pixel};

/// Where procedural images live.
const PROCEDURAL_PREFIX: &str = "procedural/";

/// The scheme's registration flags.
pub(super) fn scheme_options() -> i32 {
	[
		SchemeOptions::STANDARD,
		SchemeOptions::SECURE,
		SchemeOptions::CORS_ENABLED,
		SchemeOptions::FETCH_ENABLED,
	]
	.iter()
	.map(|o| o.get_raw() as i32)
	.fold(0, |all, o| all | o)
}

wrap_resource_handler! {
	struct CouiResource {
		files: Arc<UiFiles>,
		shared: Arc<Shared>,
		body: Arc<Mutex<Option<Body>>>,
	}

	impl ResourceHandler {
		fn open(
			&self,
			request: Option<&mut Request>,
			handle_request: Option<&mut ::std::os::raw::c_int>,
			callback: Option<&mut Callback>,
		) -> ::std::os::raw::c_int {
			let Some(request) = request else { return 0 };
			let url = CefString::from(&request.url()).to_string();
			let path = url_to_path(&url).unwrap_or_default();

			if let (Some(rest), Some(callback)) = (path.strip_prefix(PROCEDURAL_PREFIX), callback) {
				let (slot, callback) = (self.body.clone(), callback.clone());
				let responder = Responder::new(blank, move |body| {
					*slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(body);
					callback.cont();
				});
				self.shared.send(Event::Procedural { path: rest.to_string(), responder });
				// Answered later, by the game thread.
				if let Some(handle) = handle_request {
					*handle = 0;
				}
				return 1;
			}

			if let Some(handle) = handle_request {
				*handle = 1;
			}
			let body = self.files.read(&path).map(|data| {
				let mime = mime_type(&path);
				// A tiled background's picture, which Coherent draws upside down.
				let data = if mime == "image/png" && upside_down::asked_for(&url) {
					upside_down::upside_down(&data).unwrap_or(data)
				} else if whole_pixel::WANTED && mime.starts_with("font/") {
					// Glyphs advance by whole pixels, as Coherent's do (macOS).
					whole_pixel::variant(&data).unwrap_or(data)
				} else {
					data
				};
				Body::new(mime, data)
			});
			match body {
				Some(body) => {
					if body.mime.starts_with("image/") {
						self.shared.images_served.fetch_add(1, Ordering::Relaxed);
					}
					*self.body.lock().unwrap_or_else(|e| e.into_inner()) = Some(body);
					1
				}
				// A failed load (the original's "Unable to open file").
				None => {
					self.shared.loads_finished.fetch_add(1, Ordering::Relaxed);
					0
				}
			}
		}

		fn response_headers(
			&self,
			response: Option<&mut Response>,
			response_length: Option<&mut i64>,
			_redirect_url: Option<&mut CefString>,
		) {
			let body = self.body.lock().unwrap_or_else(|e| e.into_inner());
			let Some(body) = body.as_ref() else { return };
			if let Some(response) = response {
				response.set_status(200);
				response.set_status_text(Some(&"OK".into()));
				response.set_mime_type(Some(&body.mime.into()));
				response.set_header_by_name(
					Some(&"Access-Control-Allow-Origin".into()),
					Some(&"*".into()),
					1,
				);
			}
			if let Some(length) = response_length {
				*length = body.data.len() as i64;
			}
		}

		fn read(
			&self,
			data_out: *mut u8,
			bytes_to_read: ::std::os::raw::c_int,
			bytes_read: Option<&mut ::std::os::raw::c_int>,
			_callback: Option<&mut ResourceReadCallback>,
		) -> ::std::os::raw::c_int {
			let mut body = self.body.lock().unwrap_or_else(|e| e.into_inner());
			let Some(body) = body.as_mut() else { return 0 };
			let n = (body.data.len() - body.offset).min(bytes_to_read.max(0) as usize);
			if n > 0 && !data_out.is_null() {
				// SAFETY: CEF hands us a writable buffer of `bytes_to_read` bytes.
				unsafe { std::ptr::copy_nonoverlapping(body.data[body.offset..].as_ptr(), data_out, n) };
				body.offset += n;
			}
			if let Some(read) = bytes_read {
				*read = n as i32;
			}
			// 0 with 0 bytes read means the response is complete.
			if n == 0 {
				self.shared.loads_finished.fetch_add(1, Ordering::Relaxed);
			}
			(n > 0) as i32
		}

		fn cancel(&self) {}
	}
}

wrap_scheme_handler_factory! {
	pub(super) struct CouiFactory {
		files: Arc<UiFiles>,
		shared: Arc<Shared>,
	}

	impl SchemeHandlerFactory {
		fn create(
			&self,
			_browser: Option<&mut Browser>,
			_frame: Option<&mut Frame>,
			_scheme_name: Option<&CefString>,
			_request: Option<&mut Request>,
		) -> Option<ResourceHandler> {
			Some(CouiResource::new(self.files.clone(), self.shared.clone(), Arc::new(Mutex::new(None))))
		}
	}
}
