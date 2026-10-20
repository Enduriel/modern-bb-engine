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
//! - Page → game: a blocking `call` on the bridge's socket ([`bridge_wire`]: a native
//!   function of the renderer's, not a network request), which reaches the game thread
//!   as an [`Event::Request`]. The game runs it ([`UiHost::serve`]); if that calls into
//!   the page, the call travels as the answer (`{"nested": entry}`) and its result
//!   comes back as the next request (`resume`), because a renderer blocked in a call
//!   can't receive process messages.
//!
//! The page runs in its own process, so its own tasks (animation frames, timers,
//! input) could run while the game runs a script, and call into the game in the middle
//! of one of the script's calls into the page. The original can't do that: its UI runs
//! on the game's thread, between the game's scripts. So the page is **parked** while
//! the game runs scripts ([`UiHost::park`]): it waits in a bridge request
//! (`park`) that the game answers when it lets it go on, and the game's calls
//! reach it as that request's answers, as while it waits in a call.
//!
//! A page that doesn't answer in time makes the call fail (and the game carry on)
//! instead of hanging. Every process runs the same binary: call [`run_subprocess`]
//! first thing in `main`.

mod app;
mod bridge_client;
mod bridge_server;
mod bridge_wire;
mod client;
mod damage;
mod fonts;
mod frame;
pub mod goldens;
mod input;
mod ipc;
mod live_input;
mod memory;
mod page_frames;
mod page_log;
mod paint_stats;
mod renderer;
mod responder;
mod scheme;
mod session;
mod session_input;
mod session_play;
mod targets;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use cef::args::Args;
use cef::*;

use client::Shared;
pub use client::Event;
pub use damage::PaintRect;
pub use frame::FrameBuffer;
pub use input::{Key, WHEEL_DELTA_PER_NOTCH};
use ipc::Arg;
pub use live_input::{Modifiers, MouseButton};
pub use bridge_server::RequestKind;
pub use responder::Responder;
pub use page_frames::PageFrames;
pub use page_log::{Echo, LogLine};
pub use paint_stats::PaintStats;
pub use session::UiSession;
pub use session_play::ProceduralRequest;

use crate::bridge::{Reply, Transport};
use crate::coui::{MAIN_PAGE, SCHEME, UiFiles};
use crate::json::Json;

/// Where the build found the CEF distribution (`libcef.so`, its resources, locales).
const CEF_DIR: &str = env!("BB_CEF_DIR");

/// Where CEF's resources and locales are: beside the executable in a release package
/// (`bb-package` lays them out there, `icudtl.dat` among them), otherwise where the
/// build found the distribution.
fn cef_dir() -> PathBuf {
	let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
	exe_dir.filter(|dir| dir.join("icudtl.dat").is_file()).unwrap_or_else(|| PathBuf::from(CEF_DIR))
}

/// How long the game waits for the page to answer one call (time spent serving the
/// page's own calls back into the game doesn't count).
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs one of the page's calls into the game and returns the game's reply.
pub type Serve<'a> = dyn FnMut(&Json) -> Reply + 'a;

/// Run this process as a CEF child process (renderer, GPU, utility…) if it is one,
/// returning its exit code; `None` in the main (browser) process, which it prepares
/// for the page (its text laid out in whole pixels, [`fonts`]). Call it first thing
/// in `main`, before any other thread exists, and exit with the code it returns.
pub fn run_subprocess() -> Option<i32> {
	let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
	let args = Args::new();
	let command_line = args.as_cmd_line()?;
	if command_line.has_switch(Some(&"type".into())) == 0 {
		// SAFETY: first thing in `main` (this function's contract), so no other thread
		// exists yet.
		if let Err(e) = unsafe { fonts::hint_fully() } {
			eprintln!("bb-ui: the page's text won't be laid out in whole pixels: {e}");
		}
		return None;
	}
	let mut app = app::host_app(None, None, None);
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
	/// When the page makes its frames (and paints what changed): on Chromium's clock by
	/// default, in step with the game's frames for the game in a window.
	pub frames: PageFrames,
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
	/// How units move in a battle: the windowed game passes
	/// [`BattleMotion::Timed`](squirrel_bb::BattleMotion), as the original's units move
	/// (tests keep the default, a phase a call).
	pub battle_motion: squirrel_bb::BattleMotion,
	/// Mount the install's mods (every `.zip` in `data/`), as the original loads them,
	/// for the page and the scripts alike. Off by default (vanilla).
	pub mods: bool,
	/// Mod archives (`.zip`) mounted after those, for the page and the scripts alike,
	/// e.g. a test's own mod.
	pub extra_mods: Vec<PathBuf>,
	/// The scripts' `isSteamBuild()`: whether the install is Steam's (the windowed game
	/// asks `bb-steam`'s rule). True by default, as the harness's engines are.
	pub steam_build: bool,
}

impl Default for HostConfig {
	fn default() -> Self {
		HostConfig {
			url: MAIN_PAGE.to_string(),
			width: 1920,
			height: 1080,
			frames: PageFrames::Clock(30),
			cache_dir: std::env::temp_dir().join(format!("bb-ui-{}", std::process::id())),
			devtools_port: None,
			exact_clock: squirrel_bb::ExactClock::Frame,
			battle_camera: squirrel_bb::BattleCamera::Instant,
			battle_motion: squirrel_bb::BattleMotion::Stepped,
			mods: false,
			extra_mods: Vec::new(),
			steam_build: true,
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
	/// The request the page is blocked in: its call into the game that is being
	/// served, or its park.
	pending: RefCell<Option<Responder>>,
	/// Whether the page is parked (its park request is `pending`, or out carrying a
	/// call into the page).
	parked: Cell<bool>,
	/// What went wrong between the game and the page, for the owner to report.
	notes: RefCell<Vec<String>>,
	next_request: Cell<i32>,
	/// Where the mouse was last sent, if anywhere.
	pointer: Cell<Option<(i32, i32)>>,
	/// Mouse buttons held (`EVENTFLAG_*_MOUSE_BUTTON` bits), which moves carry.
	held: Cell<u32>,
	/// When the page's frames begin (see [`PageFrames`]).
	frame_clock: page_frames::FrameClock,
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
		// Bound before CEF starts: the renderer is told where it is as it launches.
		let bridge = bridge_wire::Listener::bind().map_err(|e| format!("the bridge's socket: {e}"))?;
		let mut app = app::host_app(Some(ready_tx), config.devtools_port, Some(bridge.endpoint().to_switch()));

		let path = |p: PathBuf| CefString::from(p.to_string_lossy().as_ref());
		let cef_dir = cef_dir();
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
			frame: Mutex::new(Default::default()),
			size: Mutex::new((config.width as i32, config.height as i32)),
			scale: Mutex::new(1.0),
			images_served: AtomicU64::new(0),
			paint_counters: Default::default(),
		});
		bridge_server::start(bridge, shared.clone());
		let mut factory = scheme::CouiFactory::new(files, shared.clone());
		if register_scheme_handler_factory(Some(&SCHEME.into()), None, Some(&mut factory)) != 1 {
			return Err("could not register the coui:// scheme".into());
		}
		let host = UiHost {
			shared: shared.clone(),
			events,
			deferred: RefCell::new(VecDeque::new()),
			pending: RefCell::new(None),
			parked: Cell::new(false),
			notes: RefCell::new(Vec::new()),
			next_request: Cell::new(1),
			pointer: Cell::new(None),
			held: Cell::new(0),
			frame_clock: page_frames::FrameClock::new(config.frames),
			cache_dir: config.cache_dir,
			_args: args,
		};

		let mut window_info = WindowInfo::default().set_as_windowless(Default::default());
		window_info.external_begin_frame_enabled = config.frames.external().into();
		// The generic font families are the original's, Windows' defaults (probed,
		// `probes/font_families.js`): `monospace` is Courier New there, where Linux's
		// default is DejaVu Sans Mono (a pixel taller a line: Modern Hooks' popup, and
		// MSU's `monospace` text). Fontconfig gives the metric-compatible fonts here.
		let browser_settings = BrowserSettings {
			windowless_frame_rate: config.frames.clock_rate(),
			background_color: 0,
			standard_font_family: "Times New Roman".into(),
			serif_font_family: "Times New Roman".into(),
			sans_serif_font_family: "Arial".into(),
			fixed_font_family: "Courier New".into(),
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

	/// Park the page (see the module notes): it waits in a request the game answers
	/// when it [`unpark`](Self::unpark)s it, and the game's calls into it meanwhile
	/// travel as that request's answers. Its own tasks (animation frames, timers,
	/// input) don't run until then. Waits for the page to finish what it's doing,
	/// serving its calls into the game meanwhile. Nothing to do if the page is parked
	/// already, or blocked in a call the game is serving.
	pub fn park(&self, serve: &mut Serve) -> Result<(), String> {
		if self.parked.get() || self.pending.borrow().is_some() {
			return Ok(());
		}
		let id = self.next_id();
		self.send(ipc::PARK, &[Arg::Int(id)])?;
		let mut deadline = Instant::now() + CALL_TIMEOUT;
		loop {
			let late = || format!("the page did not park within {CALL_TIMEOUT:?}");
			let left = deadline
				.checked_duration_since(Instant::now())
				.ok_or_else(late)?;
			match self.events.recv_timeout(left).map_err(|_| late())? {
				Event::Request {
					kind: RequestKind::Park,
					responder,
					..
				} => {
					self.pending.replace(Some(responder));
					self.parked.set(true);
					return Ok(());
				}
				// No bridge in the page yet: none of its scripts run.
				Event::Reply { id: reply, .. } if reply == id => return Ok(()),
				Event::Request {
					kind: RequestKind::Call | RequestKind::Task,
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

	/// Let the parked page go on (see [`park`](Self::park)).
	pub fn unpark(&self) {
		if self.parked.replace(false) {
			if let Some(responder) = self.pending.take() {
				responder.respond(r#"{"done":true}"#.into());
			}
		}
	}

	/// Whether the page is parked.
	pub fn is_parked(&self) -> bool {
		self.parked.get()
	}

	/// The game discarded a scene (a battle's end, the campaign left, a save loaded):
	/// the page gives back what it kept for it ([`memory`]).
	pub fn release_memory(&self) {
		if let Some(host) = self.browser_host() {
			let frame = host.browser().and_then(|b| b.main_frame());
			memory::release(&host, frame.as_ref());
		}
	}

	/// How many images the page has loaded from the game's files so far (each fetch
	/// counts, not each picture).
	pub fn images_served(&self) -> u64 {
		self.shared.images_served.load(Ordering::Relaxed)
	}

	/// What went wrong between the game and the page since the last call: e.g. one of
	/// the page's own tasks calling into the game while a script's call waited on the
	/// page, which parking prevents.
	pub fn take_notes(&self) -> Vec<String> {
		self.notes.take()
	}

	fn next_id(&self) -> i32 {
		let id = self.next_request.get();
		self.next_request.set(id.wrapping_add(1));
		id
	}

	/// Run `text` in the page as `kind` ([`ipc::CALL`] or [`ipc::EVAL`]) and wait for
	/// its result, serving the page's calls into the game meanwhile.
	fn run_in_page(&self, kind: &str, text: &str, serve: &mut Serve) -> Result<String, String> {
		// If the page waits (parked, or blocked in a call into the game), this is a
		// nested call: it travels as that request's answer.
		let nested = match self.pending.take() {
			Some(responder) => {
				let entry = if kind == ipc::EVAL {
					Json::Array(vec![Json::Str("e".into()), Json::Str(text.into())]).to_text()
				} else {
					text.to_string()
				};
				responder.respond(format!("{{\"nested\":{entry}}}"));
				true
			}
			None => false,
		};
		let id = self.next_id();
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
					kind: kind @ (RequestKind::Call | RequestKind::Task),
					body,
					responder,
				} => {
					if kind == RequestKind::Task && !nested {
						self.notes.borrow_mut().push(format!(
							"the page's own task called the game ({}) while the game's call into the page waited",
							body.chars().take(80).collect::<String>()
						));
					}
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

	/// Lay the page out at `width` × `height` CSS pixels from now on, drawn at `scale`
	/// screen pixels each. A UI scale is a page zoom in the original: the page lays out
	/// at the screen size divided by the scale and sees a window resize, and is drawn at
	/// the screen's resolution, laid out on its pixels (the scale is the browser's device
	/// scale factor here).
	pub fn resize(&self, width: i32, height: i32, scale: f64) {
		*self.shared.size.lock().unwrap_or_else(|e| e.into_inner()) = (width, height);
		let rescaled = {
			let mut current = self.shared.scale.lock().unwrap_or_else(|e| e.into_inner());
			let changed = *current != scale;
			*current = scale;
			changed
		};
		if let Some(host) = self.browser_host() {
			if rescaled {
				host.notify_screen_info_changed();
			}
			host.was_resized();
		}
	}

	/// Screen pixels per CSS pixel (see [`UiHost::resize`]).
	pub fn scale(&self) -> f64 {
		self.shared.scale()
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
		self.with_frame(|f| FrameBuffer {
			width: f.width,
			height: f.height,
			bgra: f.bgra.clone(),
			paints: f.paints,
		})
	}

	/// Run `f` on the latest painted frame, without copying it; `None` before the first.
	pub fn with_frame<R>(&self, f: impl FnOnce(&FrameBuffer) -> R) -> Option<R> {
		let painted = self.shared.frame.lock().unwrap_or_else(|e| e.into_inner());
		painted.frame.as_ref().map(f)
	}

	/// Run `f` on the latest painted frame and the parts of it that changed since the
	/// last call (none, if nothing did), which then count as taken: for the one owner
	/// who keeps a copy of the frame up to date (the game's GPU texture of it). The
	/// first call after the first paint, or after the view changed size, has all of it.
	/// `None` before the first paint.
	pub fn take_damage<R>(&self, f: impl FnOnce(&FrameBuffer, &[PaintRect]) -> R) -> Option<R> {
		let mut painted = self.shared.frame.lock().unwrap_or_else(|e| e.into_inner());
		let painted = &mut *painted;
		let result = painted.frame.as_ref().map(|frame| f(frame, painted.damage.rects()));
		painted.damage.clear();
		result
	}

	/// The painted page's alpha at `(x, y)` (painted pixels: the screen's, see
	/// [`UiHost::resize`]): 0 outside it, or before the first paint.
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
		self.with_frame(|f| f.paints).unwrap_or(0)
	}

	/// The game drew a frame: with [`PageFrames::Game`], the page makes its next frame
	/// now (and paints what changed by the next game frame). Nothing to do on Chromium's
	/// clock.
	pub fn begin_frame(&self) {
		if !self.frame_clock.external() {
			return;
		}
		if let Some(host) = self.browser_host() {
			host.send_external_begin_frame();
			self.frame_clock.began(Instant::now());
		}
	}

	/// A session waits on the page without drawing game frames: with
	/// [`PageFrames::Game`], begin a frame if none began for a waiting frame's time, so
	/// the page's animations go on while it waits.
	pub fn pace(&self) {
		if self.frame_clock.due(Instant::now()) {
			self.begin_frame();
		}
	}

	/// What the page's paints have cost on our side so far.
	pub fn paint_stats(&self) -> PaintStats {
		self.shared.paint_counters.get()
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
