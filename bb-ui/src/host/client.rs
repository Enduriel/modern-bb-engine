//! The browser-process handlers of the page: its paints (GPU buffers, handed to the
//! owner's [`PageSink`]), console and load reporting, the browser's lifetime, and messages from the page.
//!
//! They run on CEF's own threads and only hand events to the game thread through
//! [`Shared`]; nothing here touches the Squirrel VM.

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, mpsc};

use cef::*;

use super::damage::{Damage, PaintRect};
use super::frame::PaintState;
use super::ipc;
use super::paint_sink::{PageSink, PaintBuffer, SharedPaint};
use super::paint_stats::PaintCounters;
use super::bridge_server::RequestKind;
use super::responder::Responder;
use crate::clipboard::Clipboard;

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
	/// The page couldn't be opened: why.
	OpenFailed(String),
	/// The browser is gone (after a close).
	Closed,
	/// The renderer process died.
	RendererGone(String),
}

/// State shared between the CEF threads and the game thread.
pub(super) struct Shared {
	pub events: Mutex<mpsc::Sender<Event>>,
	pub browser: Mutex<Option<Browser>>,
	/// Where the page's paints go (the owner's copy of the frame, on the GPU).
	pub sink: Arc<dyn PageSink>,
	/// The latest frame's size and paint count, and the alpha read back from it.
	pub frame: Mutex<PaintState>,
	/// Why the last paint that couldn't be taken couldn't.
	pub paint_error: Mutex<Option<String>>,
	/// The view's width and height (CSS pixels); see [`UiHost::resize`](super::UiHost::resize).
	pub size: Mutex<(i32, i32)>,
	/// Screen pixels per CSS pixel: the UI scale, which the page is drawn at (see
	/// [`UiHost::resize`](super::UiHost::resize)).
	pub scale: Mutex<f64>,
	/// How many images `coui://` has served from the game's files.
	pub images_served: AtomicU64,
	/// How many `coui://` loads have finished (answered in full, or failed): the page
	/// has a response to take in at its next turn ([`super::page_rest`]).
	pub loads_finished: AtomicU64,
	/// What the paints cost on our side.
	pub paint_counters: PaintCounters,
	/// What the page copies to and pastes from (see [`crate::clipboard`]).
	pub clipboard: Arc<dyn Clipboard>,
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

		fn on_accelerated_paint(
			&self,
			_browser: Option<&mut Browser>,
			type_: PaintElementType,
			dirty_rects: Option<&[Rect]>,
			info: Option<&AcceleratedPaintInfo>,
		) {
			if type_ != PaintElementType::VIEW {
				return;
			}
			let Some(info) = info else { return };
			let (width, height) = (info.extra.coded_size.width, info.extra.coded_size.height);
			let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else { return };
			if width == 0 || height == 0 {
				return;
			}
			let started = std::time::Instant::now();
			// The changed parts inside the frame, merged where they meet; none said: all.
			let mut damage = Damage::default();
			for r in dirty_rects.unwrap_or_default() {
				if let Some(rect) = PaintRect::clipped(r.x, r.y, r.width, r.height, width, height) {
					damage.add(rect);
				}
			}
			if damage.rects().is_empty() {
				damage.add(PaintRect::whole(width, height));
			}
			let dirty = damage.rects();
			let taken = take_paint(&*self.shared.sink, info, width, height, dirty);
			let damaged = dirty.iter().map(PaintRect::area).sum();
			match taken {
				Ok(()) => {
					self.shared.frame.lock().unwrap_or_else(|e| e.into_inner()).painted(width, height, dirty);
					self.shared.paint_counters.add(started.elapsed(), damaged);
				}
				Err(e) => {
					if self.shared.paint_counters.failed() <= 3 {
						eprintln!("bb-ui: a paint of the page couldn't be taken: {e}");
					}
					*self.shared.paint_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
				}
			}
		}

		fn on_paint(
			&self,
			_browser: Option<&mut Browser>,
			_type_: PaintElementType,
			_dirty_rects: Option<&[Rect]>,
			_buffer: *const u8,
			_width: ::std::os::raw::c_int,
			_height: ::std::os::raw::c_int,
		) {
			// Only if Chromium fell back to compositing in software (its GPU process
			// failed): there is no software path to take this, so say why nothing shows.
			let e = "Chromium painted the page in memory, not on the GPU (its GPU process failed?); \
				the page can't be shown"
				.to_string();
			if self.shared.paint_counters.failed() <= 3 {
				eprintln!("bb-ui: {e}");
			}
			*self.shared.paint_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
		}
	}
}

/// Hand the paint `info` (its buffer valid only during this call) to `sink`.
#[cfg(target_os = "linux")]
fn take_paint(sink: &dyn PageSink, info: &AcceleratedPaintInfo, width: u32, height: u32, dirty: &[PaintRect]) -> Result<(), String> {
	use super::paint_sink::DmaBufPlane;
	use std::os::fd::BorrowedFd;

	let count = usize::try_from(info.plane_count).unwrap_or(0).min(info.planes.len());
	let mut planes = Vec::with_capacity(count);
	for plane in &info.planes[..count] {
		if plane.fd < 0 {
			return Err(format!("the paint's plane has no buffer (fd {})", plane.fd));
		}
		planes.push(DmaBufPlane {
			// SAFETY: CEF's fd, open for the duration of this callback (its contract);
			// the borrow doesn't outlive `planes`, which is dropped before returning.
			fd: unsafe { BorrowedFd::borrow_raw(plane.fd) },
			stride: plane.stride,
			offset: plane.offset,
			size: plane.size,
		});
	}
	sink.paint(&SharedPaint {
		width,
		height,
		bgra: info.format == ColorType::BGRA_8888,
		buffer: PaintBuffer::DmaBuf { planes: &planes, modifier: info.modifier },
		dirty,
	})
}

/// Hand the paint `info` (its IOSurface valid only during this call) to `sink`.
#[cfg(target_os = "macos")]
fn take_paint(sink: &dyn PageSink, info: &AcceleratedPaintInfo, width: u32, height: u32, dirty: &[PaintRect]) -> Result<(), String> {
	use super::paint_sink::IOSurface;

	let surface = std::ptr::NonNull::new(info.shared_texture_io_surface).ok_or("the paint has no IOSurface")?;
	sink.paint(&SharedPaint {
		width,
		height,
		bgra: info.format == ColorType::BGRA_8888,
		// SAFETY: CEF's surface, valid for the duration of this callback (its contract);
		// the paint doesn't outlive the call.
		buffer: PaintBuffer::IOSurface(unsafe { IOSurface::new(surface) }),
		dirty,
	})
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn take_paint(_sink: &dyn PageSink, _info: &AcceleratedPaintInfo, _width: u32, _height: u32, _dirty: &[PaintRect]) -> Result<(), String> {
	Err("the UI page's shared GPU textures aren't implemented on this operating system yet".into())
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
		keys: Option<KeyboardHandler>,
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

		fn keyboard_handler(&self) -> Option<KeyboardHandler> {
			self.keys.clone()
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
		super::edit_keys::keyboard_handler(),
	)
}
