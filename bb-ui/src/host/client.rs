//! The browser-process handlers of the page: painting into an offscreen buffer,
//! console and load reporting, the browser's lifetime, and messages from the page.
//!
//! They run on CEF's own threads and only hand events to the game thread through
//! [`Shared`]; nothing here touches the Squirrel VM.

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, mpsc};

use cef::*;

use super::ipc;
use super::bridge_server::RequestKind;
use super::responder::Responder;

/// What the CEF threads report to the game thread.
#[derive(Debug)]
pub enum Event {
	/// The page is blocked in a call into the game until `responder` answers: an
	/// `engine.call` (`[name, requestId, args...]` as JSON text), or the result of a
	/// call the game made into the page meanwhile.
	Request {
		kind: RequestKind,
		body: String,
		responder: Responder,
	},
	/// The answer to a call or eval the game sent: its JSON text, or why it failed.
	Reply {
		id: i32,
		result: Result<String, String>,
	},
	/// A `console.*` message from the page.
	Console {
		level: String,
		message: String,
		source: String,
		line: i32,
	},
	/// The main frame finished loading `url` with HTTP `status`.
	LoadEnd { url: String, status: i32 },
	/// The main frame failed to load `url`.
	LoadError { url: String, error: String },
	/// The page loads a procedural image (`coui://procedural/<path>`); `path` is still
	/// percent-encoded. The game answers it through `responder`.
	Procedural { path: String, responder: Responder },
	/// The browser exists (it can receive messages).
	Created,
	/// The browser is gone (after a close).
	Closed,
	/// The renderer process died.
	RendererGone(String),
}

/// The latest painted frame of the page.
pub struct FrameBuffer {
	pub width: u32,
	pub height: u32,
	/// BGRA, rows top to bottom.
	pub bgra: Vec<u8>,
	/// How many paints have happened so far.
	pub paints: u64,
}

/// State shared between the CEF threads and the game thread.
pub(super) struct Shared {
	pub events: Mutex<mpsc::Sender<Event>>,
	pub browser: Mutex<Option<Browser>>,
	pub frame: Mutex<Option<FrameBuffer>>,
	/// The view's width and height (CSS pixels); see [`UiHost::resize`](super::UiHost::resize).
	pub size: Mutex<(i32, i32)>,
	/// Screen pixels per CSS pixel: the UI scale, which the page is drawn at (see
	/// [`UiHost::resize`](super::UiHost::resize)).
	pub scale: Mutex<f64>,
	/// How many images `coui://` has served from the game's files.
	pub images_served: AtomicU64,
}

impl Shared {
	pub fn size(&self) -> (i32, i32) {
		*self.size.lock().unwrap_or_else(|e| e.into_inner())
	}

	pub fn scale(&self) -> f64 {
		*self.scale.lock().unwrap_or_else(|e| e.into_inner())
	}

	pub fn send(&self, event: Event) {
		let _ = self
			.events
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.send(event);
	}
}

wrap_render_handler! {
	struct OffscreenView {
		shared: Arc<Shared>,
	}

	impl RenderHandler {
		fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
			if let Some(rect) = rect {
				let (width, height) = self.shared.size();
				*rect = Rect { x: 0, y: 0, width, height };
			}
		}

		fn screen_info(
			&self,
			_browser: Option<&mut Browser>,
			screen_info: Option<&mut ScreenInfo>,
		) -> ::std::os::raw::c_int {
			let Some(info) = screen_info else { return 0 };
			info.device_scale_factor = self.shared.scale() as f32;
			let (width, height) = self.shared.size();
			let rect = Rect { x: 0, y: 0, width, height };
			info.rect = rect.clone();
			info.available_rect = rect;
			1
		}

		fn on_paint(
			&self,
			_browser: Option<&mut Browser>,
			type_: PaintElementType,
			_dirty_rects: Option<&[Rect]>,
			buffer: *const u8,
			width: ::std::os::raw::c_int,
			height: ::std::os::raw::c_int,
		) {
			if type_ != PaintElementType::VIEW || buffer.is_null() || width <= 0 || height <= 0 {
				return;
			}
			let len = width as usize * height as usize * 4;
			// SAFETY: CEF passes a BGRA buffer of `width * height` pixels.
			let pixels = unsafe { std::slice::from_raw_parts(buffer, len) };
			let mut frame = self.shared.frame.lock().unwrap_or_else(|e| e.into_inner());
			let paints = frame.as_ref().map_or(0, |f| f.paints) + 1;
			match frame.as_mut() {
				Some(f) if f.bgra.len() == len => f.bgra.copy_from_slice(pixels),
				_ => {
					*frame = Some(FrameBuffer {
						width: width as u32,
						height: height as u32,
						bgra: pixels.to_vec(),
						paints: 0,
					})
				}
			}
			if let Some(f) = frame.as_mut() {
				f.width = width as u32;
				f.height = height as u32;
				f.paints = paints;
			}
		}
	}
}

wrap_display_handler! {
	struct ConsoleRelay {
		shared: Arc<Shared>,
	}

	impl DisplayHandler {
		fn on_console_message(
			&self,
			_browser: Option<&mut Browser>,
			level: LogSeverity,
			message: Option<&CefString>,
			source: Option<&CefString>,
			line: ::std::os::raw::c_int,
		) -> ::std::os::raw::c_int {
			let level = match level {
				LogSeverity::ERROR | LogSeverity::FATAL => "error",
				LogSeverity::WARNING => "warning",
				_ => "info",
			};
			self.shared.send(Event::Console {
				level: level.to_string(),
				message: message.map(|m| m.to_string()).unwrap_or_default(),
				source: source.map(|s| s.to_string()).unwrap_or_default(),
				line,
			});
			// Handled: keep it out of CEF's own log.
			1
		}
	}
}

wrap_load_handler! {
	struct LoadRelay {
		shared: Arc<Shared>,
	}

	impl LoadHandler {
		fn on_load_end(
			&self,
			_browser: Option<&mut Browser>,
			frame: Option<&mut Frame>,
			http_status_code: ::std::os::raw::c_int,
		) {
			let Some(frame) = frame else { return };
			if frame.is_main() != 0 {
				let url = CefString::from(&frame.url()).to_string();
				self.shared.send(Event::LoadEnd { url, status: http_status_code });
			}
		}

		fn on_load_error(
			&self,
			_browser: Option<&mut Browser>,
			frame: Option<&mut Frame>,
			_error_code: Errorcode,
			error_text: Option<&CefString>,
			failed_url: Option<&CefString>,
		) {
			if frame.is_some_and(|f| f.is_main() != 0) {
				self.shared.send(Event::LoadError {
					url: failed_url.map(|u| u.to_string()).unwrap_or_default(),
					error: error_text.map(|e| e.to_string()).unwrap_or_default(),
				});
			}
		}
	}
}

wrap_life_span_handler! {
	struct Lifetime {
		shared: Arc<Shared>,
	}

	impl LifeSpanHandler {
		fn on_before_popup(
			&self,
			_browser: Option<&mut Browser>,
			_frame: Option<&mut Frame>,
			_popup_id: ::std::os::raw::c_int,
			_target_url: Option<&CefString>,
			_target_frame_name: Option<&CefString>,
			_target_disposition: WindowOpenDisposition,
			_user_gesture: ::std::os::raw::c_int,
			_popup_features: Option<&PopupFeatures>,
			_window_info: Option<&mut WindowInfo>,
			_client: Option<&mut Option<Client>>,
			_settings: Option<&mut BrowserSettings>,
			_extra_info: Option<&mut Option<DictionaryValue>>,
			_no_javascript_access: Option<&mut ::std::os::raw::c_int>,
		) -> ::std::os::raw::c_int {
			// The UI never opens windows.
			1
		}

		fn on_after_created(&self, browser: Option<&mut Browser>) {
			*self.shared.browser.lock().unwrap_or_else(|e| e.into_inner()) = browser.cloned();
			self.shared.send(Event::Created);
		}

		fn on_before_close(&self, _browser: Option<&mut Browser>) {
			*self.shared.browser.lock().unwrap_or_else(|e| e.into_inner()) = None;
			self.shared.send(Event::Closed);
		}
	}
}

wrap_request_handler! {
	struct CrashRelay {
		shared: Arc<Shared>,
	}

	impl RequestHandler {
		fn on_render_process_terminated(
			&self,
			_browser: Option<&mut Browser>,
			_status: TerminationStatus,
			error_code: ::std::os::raw::c_int,
			error_string: Option<&CefString>,
		) {
			let why = error_string.map(|e| e.to_string()).unwrap_or_default();
			self.shared.send(Event::RendererGone(format!("{why} (code {error_code})")));
		}
	}
}

wrap_context_menu_handler! {
	struct NoContextMenu;

	impl ContextMenuHandler {
		fn run_context_menu(
			&self,
			_browser: Option<&mut Browser>,
			_frame: Option<&mut Frame>,
			_params: Option<&mut ContextMenuParams>,
			_model: Option<&mut MenuModel>,
			callback: Option<&mut RunContextMenuCallback>,
		) -> ::std::os::raw::c_int {
			// Coherent has no context menu (and an offscreen view can't show Chromium's).
			// The page has had the right-click by now.
			if let Some(callback) = callback {
				callback.cancel();
			}
			1
		}
	}
}

wrap_client! {
	pub(super) struct PageClient {
		shared: Arc<Shared>,
		view: RenderHandler,
		console: DisplayHandler,
		load: LoadHandler,
		lifetime: LifeSpanHandler,
		requests: RequestHandler,
		menu: ContextMenuHandler,
	}

	impl Client {
		fn render_handler(&self) -> Option<RenderHandler> {
			Some(self.view.clone())
		}

		fn display_handler(&self) -> Option<DisplayHandler> {
			Some(self.console.clone())
		}

		fn load_handler(&self) -> Option<LoadHandler> {
			Some(self.load.clone())
		}

		fn life_span_handler(&self) -> Option<LifeSpanHandler> {
			Some(self.lifetime.clone())
		}

		fn request_handler(&self) -> Option<RequestHandler> {
			Some(self.requests.clone())
		}

		fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
			Some(self.menu.clone())
		}

		fn on_process_message_received(
			&self,
			_browser: Option<&mut Browser>,
			_frame: Option<&mut Frame>,
			_source_process: ProcessId,
			message: Option<&mut ProcessMessage>,
		) -> ::std::os::raw::c_int {
			let Some(message) = message else { return 0 };
			match ipc::name(message).as_str() {
				ipc::REPLY => {
					let text = ipc::string_arg(message, 2);
					let result = if ipc::int_arg(message, 1) != 0 { Ok(text) } else { Err(text) };
					self.shared.send(Event::Reply { id: ipc::int_arg(message, 0), result });
				}
				_ => return 0,
			}
			1
		}
	}
}

/// The client for the page's browser.
pub(super) fn page_client(shared: Arc<Shared>) -> Client {
	PageClient::new(
		shared.clone(),
		OffscreenView::new(shared.clone()),
		ConsoleRelay::new(shared.clone()),
		LoadRelay::new(shared.clone()),
		Lifetime::new(shared.clone()),
		CrashRelay::new(shared),
		NoContextMenu::new(),
	)
}
