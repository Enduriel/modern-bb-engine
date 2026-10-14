//! The Chromium host: the game's UI page in CEF, rendered offscreen.
//!
//! Threads:
//! - the **game thread** (the one that calls [`UiHost::start`]) owns the Squirrel VM
//!   and this [`UiHost`]; it is never CEF's UI thread (`multi_threaded_message_loop`);
//! - CEF's own threads run the handlers in [`client`], [`scheme`] and [`app`], which
//!   only pass [`Event`]s to the game thread (and serve `coui://` from the archives);
//! - the page runs in a renderer **process** ([`renderer`]).
//!
//! Calls between game and page run inline, as in the original: whoever calls waits
//! until the other side has run the call, and serves the other side's calls back in
//! the meantime.
//! - Game → page: a [`ipc::CALL`] process message; the game thread waits for the reply
//!   ([`UiHost::run`]).
//! - Page → game: a synchronous XHR to `coui://ui/__bb__/call`, which reaches the game
//!   thread as an [`Event::Request`]. The game runs it ([`UiHost::serve`]); if that
//!   calls into the page, the call travels as the XHR's answer (`{"nested": entry}`)
//!   and its result comes back as the next XHR (`__bb__/resume`), because a renderer
//!   blocked in an XHR can't receive process messages.
//!
//! A page that doesn't answer in time makes the call fail (and the game carry on)
//! instead of hanging. Every process runs the same binary: call [`run_subprocess`]
//! first thing in `main`.

mod app;
mod client;
pub mod goldens;
mod input;
mod ipc;
mod live_input;
mod renderer;
mod scheme;
mod session;
mod session_input;
mod session_play;
mod targets;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use cef::args::Args;
use cef::*;

use client::Shared;
pub use client::{Event, FrameBuffer};
pub use input::Key;
use ipc::Arg;
pub use live_input::{Modifiers, MouseButton};
pub use scheme::{RequestKind, Responder};
pub use session::{LogLine, UiSession};
pub use session_play::ProceduralRequest;

use crate::bridge::{Reply, Transport};
use crate::coui::{MAIN_PAGE, SCHEME, UiFiles};
use crate::json::Json;

/// Where the build found the CEF distribution (`libcef.so`, its resources, locales).
const CEF_DIR: &str = env!("BB_CEF_DIR");

/// How long the game waits for the page to answer one call (time spent serving the
/// page's own calls back into the game doesn't count).
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs one of the page's calls into the game and returns the game's reply.
pub type Serve<'a> = dyn FnMut(&Json) -> Reply + 'a;

/// Run this process as a CEF child process (renderer, GPU, utility…) if it is one,
/// returning its exit code; `None` in the main (browser) process. Call it first
/// thing in `main` and exit with the code it returns.
pub fn run_subprocess() -> Option<i32> {
	let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
	let args = Args::new();
	let command_line = args.as_cmd_line()?;
	if command_line.has_switch(Some(&"type".into())) == 0 {
		return None;
	}
	let mut app = app::host_app(None);
	Some(execute_process(
		Some(args.as_main_args()),
		Some(&mut app),
		std::ptr::null_mut(),
	))
}

/// How to run the page.
pub struct HostConfig {
	/// The page to load (the game's UI by default).
	pub url: String,
	/// The view size in pixels (the game's resolution).
	pub width: u32,
	pub height: u32,
	/// Offscreen paints per second.
	pub frame_rate: i32,
	/// A directory CEF may use for its profile and log (created if missing).
	pub cache_dir: PathBuf,
	/// Serve Chrome DevTools on `127.0.0.1:<port>` (open `chrome://inspect` in a
	/// browser). Off by default.
	pub devtools_port: Option<u16>,
	/// The clock behind `Time.getExactTime()` for the scripts this page runs with: the
	/// windowed game passes [`ExactClock::Live`], as the original's clock is (tests keep
	/// the default frame time).
	pub exact_clock: squirrel_bb::ExactClock,
	/// How the battle camera moves for those scripts: the windowed game passes
	/// [`BattleCamera::Glides`](squirrel_bb::BattleCamera), as the original's camera
	/// does (tests keep the default instant camera).
	pub battle_camera: squirrel_bb::BattleCamera,
}

impl Default for HostConfig {
	fn default() -> Self {
		HostConfig {
			url: MAIN_PAGE.to_string(),
			width: 1920,
			height: 1080,
			frame_rate: 30,
			cache_dir: std::env::temp_dir().join(format!("bb-ui-{}", std::process::id())),
			devtools_port: None,
			exact_clock: squirrel_bb::ExactClock::Frame,
			battle_camera: squirrel_bb::BattleCamera::Instant,
		}
	}
}

/// The running UI page. Lives on the game thread; dropping it closes the page and
/// shuts CEF down.
pub struct UiHost {
	shared: Arc<Shared>,
	events: mpsc::Receiver<Event>,
	/// Events set aside while a blocking call waited.
	deferred: RefCell<VecDeque<Event>>,
	/// The page's call into the game that is being served (the page is blocked in it).
	pending: RefCell<Option<Responder>>,
	next_request: Cell<i32>,
	/// Where the mouse was last sent, if anywhere.
	pointer: Cell<Option<(i32, i32)>>,
	/// Mouse buttons held (`EVENTFLAG_*_MOUSE_BUTTON` bits), which moves carry.
	held: Cell<u32>,
	cache_dir: PathBuf,
	_args: Args,
}

impl UiHost {
	/// Start CEF and open the page, serving `coui://` from `files`. Returns once the
	/// browser exists; the page itself is still loading.
	pub fn start(files: Arc<UiFiles>, config: HostConfig) -> Result<UiHost, String> {
		let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
		std::fs::create_dir_all(&config.cache_dir).map_err(|e| format!("cache dir: {e}"))?;
		let args = Args::new();
		let (ready_tx, ready_rx) = mpsc::channel();
		let mut app = app::host_app(Some(ready_tx));

		let path = |p: PathBuf| CefString::from(p.to_string_lossy().as_ref());
		let cef_dir = PathBuf::from(CEF_DIR);
		let settings = Settings {
			no_sandbox: 1,
			windowless_rendering_enabled: 1,
			multi_threaded_message_loop: 1,
			root_cache_path: path(config.cache_dir.clone()),
			log_file: path(config.cache_dir.join("cef.log")),
			log_severity: LogSeverity::WARNING,
			resources_dir_path: path(cef_dir.clone()),
			locales_dir_path: path(cef_dir.join("locales")),
			// Transparent: the UI is drawn over the game's scene.
			background_color: 0,
			remote_debugging_port: config.devtools_port.map_or(0, i32::from),
			..Default::default()
		};
		if initialize(
			Some(args.as_main_args()),
			Some(&settings),
			Some(&mut app),
			std::ptr::null_mut(),
		) != 1
		{
			return Err("CEF failed to initialize (see cef.log in the cache dir)".into());
		}
		ready_rx
			.recv_timeout(Duration::from_secs(30))
			.map_err(|_| "CEF did not finish initializing".to_string())?;

		let (events_tx, events) = mpsc::channel();
		let shared = Arc::new(Shared {
			events: Mutex::new(events_tx),
			browser: Mutex::new(None),
			frame: Mutex::new(None),
			size: Mutex::new((config.width as i32, config.height as i32)),
		});
		let mut factory = scheme::CouiFactory::new(files, shared.clone());
		if register_scheme_handler_factory(Some(&SCHEME.into()), None, Some(&mut factory)) != 1 {
			return Err("could not register the coui:// scheme".into());
		}
		let host = UiHost {
			shared: shared.clone(),
			events,
			deferred: RefCell::new(VecDeque::new()),
			pending: RefCell::new(None),
			next_request: Cell::new(1),
			pointer: Cell::new(None),
			held: Cell::new(0),
			cache_dir: config.cache_dir,
			_args: args,
		};

		let window_info = WindowInfo::default().set_as_windowless(Default::default());
		let browser_settings = BrowserSettings {
			windowless_frame_rate: config.frame_rate,
			background_color: 0,
			..Default::default()
		};
		let mut client = client::page_client(shared);
		let created = browser_host_create_browser(
			Some(&window_info),
			Some(&mut client),
			Some(&config.url.as_str().into()),
			Some(&browser_settings),
			None,
			None,
		);
		if created != 1 {
			return Err("could not create the browser".into());
		}
		host.wait_for(Duration::from_secs(30), |e| matches!(e, Event::Created))
			.ok_or("the browser was not created in time")?;
		Ok(host)
	}

	/// Every event that arrived since the last poll, in order. Doesn't wait.
	pub fn poll(&self) -> Vec<Event> {
		let mut events: Vec<Event> = self.deferred.borrow_mut().drain(..).collect();
		events.extend(self.events.try_iter());
		events
	}

	/// The next event, waiting up to `timeout` for one.
	pub fn next_event(&self, timeout: Duration) -> Option<Event> {
		if let Some(event) = self.deferred.borrow_mut().pop_front() {
			return Some(event);
		}
		self.events.recv_timeout(timeout).ok()
	}

	/// Wait up to `timeout` for an event matching `wanted` and return it; every other
	/// event is kept for the next [`poll`](Self::poll).
	pub fn wait_for(&self, timeout: Duration, wanted: impl Fn(&Event) -> bool) -> Option<Event> {
		let deadline = Instant::now() + timeout;
		loop {
			let left = deadline.checked_duration_since(Instant::now())?;
			let event = self.events.recv_timeout(left).ok()?;
			if wanted(&event) {
				return Some(event);
			}
			self.deferred.borrow_mut().push_back(event);
		}
	}

	/// Send `name(args)` to the page's main frame.
	fn send(&self, name: &str, args: &[Arg]) -> Result<(), String> {
		let browser = self
			.shared
			.browser
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.clone();
		let frame = browser
			.and_then(|b| b.main_frame())
			.ok_or("the page is gone")?;
		if ipc::send(&frame, ProcessId::RENDERER, name, args) {
			Ok(())
		} else {
			Err(format!("could not send {name}"))
		}
	}

	/// Answer the page's call into the game (an [`Event::Request`] of kind
	/// [`RequestKind::Call`], whose `body` is `[name, requestId, args...]`) with what
	/// `serve` returns. The page stays blocked until then; any call into the page made
	/// meanwhile reaches it through this request.
	pub fn serve(&self, responder: Responder, body: &str, serve: &mut Serve) {
		let outer = self.pending.replace(Some(responder));
		debug_assert!(outer.is_none(), "the page waits on one call at a time");
		let reply = match Json::parse(body) {
			Ok(message) => serve(&message),
			Err(error) => {
				eprintln!("bb-ui: unreadable call from the page: {error}");
				Reply::Void
			}
		};
		// Nested calls into the page may have swapped the request the page now waits
		// on; answer whichever it is.
		if let Some(responder) = self.pending.take() {
			responder.respond(reply.to_json().to_text());
		}
	}

	/// Run `text` in the page as `kind` ([`ipc::CALL`] or [`ipc::EVAL`]) and wait for
	/// its result, serving the page's calls into the game meanwhile.
	fn run_in_page(&self, kind: &str, text: &str, serve: &mut Serve) -> Result<String, String> {
		// If the page is blocked in a call into the game, this is a nested call: it
		// travels as that call's answer.
		let nested = match self.pending.take() {
			Some(responder) if kind == ipc::CALL => {
				responder.respond(format!("{{\"nested\":{text}}}"));
				true
			}
			Some(responder) => {
				self.pending.replace(Some(responder));
				return Err("the page is busy in a call into the game".into());
			}
			None => false,
		};
		let id = self.next_request.get();
		self.next_request.set(id.wrapping_add(1));
		if !nested {
			self.send(kind, &[Arg::Int(id), Arg::Str(text)])?;
		}

		let mut deadline = Instant::now() + CALL_TIMEOUT;
		loop {
			let left = deadline
				.checked_duration_since(Instant::now())
				.ok_or_else(|| format!("the page did not answer within {CALL_TIMEOUT:?}"))?;
			let event = self
				.events
				.recv_timeout(left)
				.map_err(|_| format!("the page did not answer within {CALL_TIMEOUT:?}"))?;
			match event {
				Event::Reply { id: reply, result } if !nested && reply == id => return result,
				Event::Request {
					kind: RequestKind::Resume,
					body,
					responder,
				} if nested => {
					self.pending.replace(Some(responder));
					return Ok(body);
				}
				Event::Request {
					kind: RequestKind::Call,
					body,
					responder,
				} => {
					self.serve(responder, &body, serve);
					deadline = Instant::now() + CALL_TIMEOUT;
				}
				Event::RendererGone(why) => {
					let error = format!("the page crashed: {why}");
					self.deferred
						.borrow_mut()
						.push_back(Event::RendererGone(why));
					return Err(error);
				}
				other => self.deferred.borrow_mut().push_back(other),
			}
		}
	}

	/// Evaluate `code` in the page (global scope) and return its value as JSON. Calls
	/// the code makes into the game go to `serve`.
	pub fn eval(&self, code: &str, serve: &mut Serve) -> Result<Json, String> {
		let reply = Json::parse(&self.run_in_page(ipc::EVAL, code, serve)?)?;
		if let Some(error) = reply.get("error") {
			return Err(error.as_str().unwrap_or("error").to_string());
		}
		Ok(reply.get("ok").cloned().unwrap_or(Json::Null))
	}

	fn browser_host(&self) -> Option<BrowserHost> {
		let browser = self
			.shared
			.browser
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.clone();
		browser.and_then(|b| b.host())
	}

	/// The view's size (CSS pixels): what the page lays out at.
	pub fn size(&self) -> (i32, i32) {
		self.shared.size()
	}

	/// Lay the page out at `width` × `height` from now on (a UI scale change resizes the
	/// view, as in the original; the page sees a window resize).
	pub fn resize(&self, width: i32, height: i32) {
		*self.shared.size.lock().unwrap_or_else(|e| e.into_inner()) = (width, height);
		if let Some(host) = self.browser_host() {
			host.was_resized();
		}
	}

	/// Where the mouse was last sent (view pixels), if anywhere.
	pub fn pointer(&self) -> Option<(i32, i32)> {
		self.pointer.get()
	}

	/// Move the mouse to `(x, y)` (view pixels).
	pub fn mouse_move(&self, x: i32, y: i32) {
		if let Some(host) = self.browser_host() {
			self.pointer.set(Some((x, y)));
			let modifiers = self.held.get();
			host.send_mouse_move_event(Some(&MouseEvent { x, y, modifiers }), 0);
		}
	}

	/// Move the mouse out of the view, if it's in it (hover ends).
	pub fn mouse_leave(&self) {
		if let (Some((x, y)), Some(host)) = (self.pointer.take(), self.browser_host()) {
			let modifiers = self.held.get();
			host.send_mouse_move_event(Some(&MouseEvent { x, y, modifiers }), 1);
		}
	}

	/// Left-click at `(x, y)`: move there, press, release.
	pub fn click(&self, x: i32, y: i32) {
		let Some(host) = self.browser_host() else {
			return;
		};
		self.pointer.set(Some((x, y)));
		let event = MouseEvent { x, y, modifiers: 0 };
		host.send_mouse_move_event(Some(&event), 0);
		host.send_mouse_click_event(Some(&event), MouseButtonType::LEFT, 0, 1);
		host.send_mouse_click_event(Some(&event), MouseButtonType::LEFT, 1, 1);
	}

	/// `button` went down or up at `(x, y)` (view pixels), as a player's mouse does it.
	/// Moves carry the buttons held, so the page sees drags.
	pub fn mouse_button(&self, x: i32, y: i32, button: MouseButton, down: bool) {
		let Some(host) = self.browser_host() else {
			return;
		};
		self.pointer.set(Some((x, y)));
		let flag = button.held_flag();
		let held = if down {
			self.held.get() | flag
		} else {
			self.held.get() & !flag
		};
		// The event itself carries the button while it goes down and until it is up.
		live_input::button(&host, x, y, button, down, self.held.get() | flag);
		self.held.set(held);
	}

	/// Key `code` (a Windows virtual-key code) went down or up in the focused element.
	pub fn key(&self, code: i32, down: bool, modifiers: Modifiers) {
		if let Some(host) = self.browser_host() {
			live_input::key(&host, code, down, modifiers);
		}
	}

	/// The keys typed `c` in the focused element (after the key went down).
	pub fn character(&self, c: char, modifiers: Modifiers) {
		if let Some(host) = self.browser_host() {
			live_input::character(&host, c, modifiers);
		}
	}

	/// Press and release `key` in the focused element.
	pub fn press_key(&self, key: Key) {
		if let Some(host) = self.browser_host() {
			input::press_key(&host, key);
		}
	}

	/// Type `text` into the focused element, a key press per character.
	pub fn type_text(&self, text: &str) {
		if let Some(host) = self.browser_host() {
			input::type_text(&host, text);
		}
	}

	/// Turn the mouse wheel `notches` times at `(x, y)`; positive scrolls up.
	pub fn wheel(&self, x: i32, y: i32, notches: i32) {
		if let Some(host) = self.browser_host() {
			self.pointer.set(Some((x, y)));
			input::wheel(&host, x, y, notches);
		}
	}

	/// A copy of the latest painted frame.
	pub fn frame(&self) -> Option<FrameBuffer> {
		let frame = self.shared.frame.lock().unwrap_or_else(|e| e.into_inner());
		frame.as_ref().map(|f| FrameBuffer {
			width: f.width,
			height: f.height,
			bgra: f.bgra.clone(),
			paints: f.paints,
		})
	}

	/// Run `f` on the latest painted frame, without copying it; `None` before the first.
	pub fn with_frame<R>(&self, f: impl FnOnce(&FrameBuffer) -> R) -> Option<R> {
		let frame = self.shared.frame.lock().unwrap_or_else(|e| e.into_inner());
		frame.as_ref().map(f)
	}

	/// The painted page's alpha at `(x, y)` (view pixels): 0 outside it, or before the
	/// first paint.
	pub fn alpha_at(&self, x: i32, y: i32) -> u8 {
		self.with_frame(|f| {
			let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
				return 0;
			};
			if x >= f.width || y >= f.height {
				return 0;
			}
			f.bgra[((y * f.width + x) * 4 + 3) as usize]
		})
		.unwrap_or(0)
	}

	/// How many frames have been painted so far.
	pub fn paints(&self) -> u64 {
		self.shared
			.frame
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.as_ref()
			.map_or(0, |f| f.paints)
	}

	/// The directory with CEF's profile and log.
	pub fn cache_dir(&self) -> &PathBuf {
		&self.cache_dir
	}
}

impl Transport for UiHost {
	fn run(&self, entry: &Json, serve: &mut Serve) -> Result<(), String> {
		self.run_in_page(ipc::CALL, &entry.to_text(), serve)
			.map(|_| ())
	}
}

impl Drop for UiHost {
	fn drop(&mut self) {
		// Release a page still waiting on the game.
		drop(self.pending.take());
		let browser = self
			.shared
			.browser
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.clone();
		if let Some(host) = browser.and_then(|b| b.host()) {
			host.close_browser(1);
			self.wait_for(Duration::from_secs(10), |e| matches!(e, Event::Closed));
		}
		shutdown();
		let _ = std::fs::remove_dir_all(&self.cache_dir);
	}
}
