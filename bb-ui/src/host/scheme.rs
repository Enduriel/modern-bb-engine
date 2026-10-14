//! The `coui://` scheme handler, on CEF's IO thread.
//!
//! - Files: served from [`UiFiles`] without involving the game thread. `coui` is
//!   registered as a standard (hierarchical, so relative URLs resolve), secure, CORS-
//!   and fetch-enabled scheme. Each first path segment is its own origin (`coui://ui`
//!   loads fonts from `coui://gfx`), so every response allows any origin, as the
//!   original's single file system did. A missing file is a network error, as in the
//!   original.
//! - The bridge: `coui://ui/__bb__/call` and `.../resume` are the page's blocking calls
//!   into the game (see `bootstrap.js`). Each becomes an [`Event::Request`] for the game
//!   thread, and the page's XHR waits until the game answers through its [`Responder`].
//! - Procedural images (`coui://procedural/tacticalentity(...)`, `worldentity(...)`):
//!   the engine composes portraits from entity sprites. Each becomes an
//!   [`Event::Procedural`] for the game thread, which answers with a PNG; when it
//!   can't (or doesn't answer), a transparent placeholder. The original also answers
//!   200 with a fallback image when composing fails, never a load error.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};

use cef::*;

use super::client::{Event, Shared};
use crate::coui::{UiFiles, mime_type, url_to_path};

/// Where the page's bridge calls go (below the page's own origin, `coui://ui`).
const BRIDGE_PREFIX: &str = "ui/__bb__/";

/// Where procedural images live.
const PROCEDURAL_PREFIX: &str = "procedural/";

/// A 1×1 transparent PNG.
fn blank_image() -> &'static [u8] {
	static IMAGE: OnceLock<Vec<u8>> = OnceLock::new();
	IMAGE.get_or_init(|| {
		let mut png = Vec::new();
		crate::png::write_rgba(&mut png, 1, 1, &[0, 0, 0, 0]).expect("writing to a Vec");
		png
	})
}

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

/// Which bridge call the page made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
	/// `engine.call(...)` from inside one of the game's calls into the page: run a
	/// handler in the game.
	Call,
	/// `engine.call(...)` from one of the page's own tasks (an animation frame, a
	/// timer, input): the same, but the game wasn't calling the page.
	Task,
	/// The result of a call the game made into the page while the page was waiting.
	Resume,
	/// The page parks: it waits until the game lets it go on (`UiHost::park`).
	Park,
}

/// One response being read.
struct Body {
	mime: &'static str,
	data: Vec<u8>,
	offset: usize,
}

/// How the game thread answers a waiting request: a bridge call, or a procedural
/// image. Answering (or dropping it unanswered, which answers "done" or the blank
/// image) lets the page continue.
pub struct Responder {
	slot: Arc<Mutex<Option<Body>>>,
	callback: Option<Callback>,
	/// What it answers if dropped unanswered.
	fallback: fn() -> Body,
}

impl Responder {
	/// Answer with `json`.
	pub fn respond(mut self, json: String) {
		self.finish(Body {
			mime: "application/json",
			data: json.into_bytes(),
			offset: 0,
		});
	}

	/// Answer with an image: `png`, or the transparent placeholder.
	pub fn respond_image(mut self, png: Option<Vec<u8>>) {
		self.finish(Body {
			mime: "image/png",
			data: png.unwrap_or_else(|| blank_image().to_vec()),
			offset: 0,
		});
	}

	fn finish(&mut self, body: Body) {
		let Some(callback) = self.callback.take() else {
			return;
		};
		*self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(body);
		callback.cont();
	}
}

impl Drop for Responder {
	fn drop(&mut self) {
		// Never leave the page waiting.
		let fallback = (self.fallback)();
		self.finish(fallback);
	}
}

/// What an unanswered bridge call answers.
fn done() -> Body {
	Body {
		mime: "application/json",
		data: br#"{"done":true}"#.to_vec(),
		offset: 0,
	}
}

/// What an unanswered procedural image answers.
fn blank() -> Body {
	Body {
		mime: "image/png",
		data: blank_image().to_vec(),
		offset: 0,
	}
}

impl std::fmt::Debug for Responder {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("Responder")
	}
}

/// The request body (a POST's bytes) as text.
fn post_body(request: &Request) -> String {
	let Some(post) = request.post_data() else {
		return String::new();
	};
	let mut elements: Vec<Option<PostDataElement>> =
		(0..post.element_count()).map(|_| None).collect();
	post.elements(Some(&mut elements));
	let mut bytes = Vec::new();
	for element in elements.into_iter().flatten() {
		let size = element.bytes_count();
		let start = bytes.len();
		bytes.resize(start + size, 0);
		let read = element.bytes(size, bytes[start..].as_mut_ptr());
		bytes.truncate(start + read);
	}
	String::from_utf8_lossy(&bytes).into_owned()
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

			if let Some(kind) = path.strip_prefix(BRIDGE_PREFIX) {
				let kind = match kind {
					"call" => RequestKind::Call,
					"task" => RequestKind::Task,
					"resume" => RequestKind::Resume,
					"park" => RequestKind::Park,
					_ => return 0,
				};
				let Some(callback) = callback else { return 0 };
				let responder = Responder {
					slot: self.body.clone(),
					callback: Some(callback.clone()),
					fallback: done,
				};
				self.shared.send(Event::Request { kind, body: post_body(request), responder });
				// Answered later, by the game thread.
				if let Some(handle) = handle_request {
					*handle = 0;
				}
				return 1;
			}

			if let (Some(rest), Some(callback)) = (path.strip_prefix(PROCEDURAL_PREFIX), callback) {
				let responder = Responder {
					slot: self.body.clone(),
					callback: Some(callback.clone()),
					fallback: blank,
				};
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
			let body = self
				.files
				.read(&path)
				.map(|data| Body { mime: mime_type(&path), data, offset: 0 });
			match body {
				Some(body) => {
					if body.mime.starts_with("image/") {
						self.shared.images_served.fetch_add(1, Ordering::Relaxed);
					}
					*self.body.lock().unwrap_or_else(|e| e.into_inner()) = Some(body);
					1
				}
				// A failed load (the original's "Unable to open file").
				None => 0,
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
