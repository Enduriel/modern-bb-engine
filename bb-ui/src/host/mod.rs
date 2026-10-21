//! The Chromium host: the game's UI page in CEF, rendered offscreen on the GPU. Its
//! paints are handed to the owner's [`PageSink`] as GPU buffers ([`paint_sink`]),
//! through a display connection ([`display`]).
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
//! reach it as that request's answers, as while it waits in a call. A page with nothing
//! to do stays parked across the game's frames ([`page_rest`]).
//!
//! A page that doesn't answer in time makes the call fail (and the game carry on)
//! instead of hanging. Every process runs the same binary: call [`run_subprocess`]
//! first thing in `main`.

mod app;
mod bridge_client;
mod bridge_server;
mod bridge_wire;
mod cef_thread;
mod click_count;
mod client;
mod damage;
mod display;
mod edit_keys;
#[cfg(target_os = "linux")]
mod fonts;
mod frame;
pub mod goldens;
mod input;
mod ipc;
mod live_input;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod mac_keys;
mod memory;
mod opening;
mod page_frames;
mod page_log;
mod page_rest;
mod paint_sink;
mod paint_stats;
mod pump_schedule;
mod renderer;
mod responder;
mod scheme;
mod session;
mod session_click_check;
mod session_input;
mod session_fps;
mod session_play;
#[cfg(target_os = "macos")]
mod system_fonts;
mod targets;
pub mod tracing;

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
pub use display::{Display, with_display};
pub use frame::FrameBuffer;
pub use input::{Key, WHEEL_DELTA_PER_NOTCH};
use ipc::Arg;
pub use live_input::{Modifiers, MouseButton, coherent_key_code, modifiers_of_mask};
pub use bridge_server::RequestKind;
pub use responder::Responder;
pub use page_frames::PageFrames;
pub use page_log::{Echo, LogLine};
#[cfg(target_os = "linux")]
pub use paint_sink::DmaBufPlane;
#[cfg(target_os = "macos")]
pub use paint_sink::IOSurface;
#[cfg(windows)]
pub use paint_sink::D3D11Texture;
pub use paint_sink::{PageSink, PaintBuffer, SharedPaint};
pub use paint_stats::PaintStats;
pub use pump_schedule::Waker;
pub use session::{NO_PAINTS, UiSession};
pub use session_play::ProceduralRequest;

use crate::bridge::{Reply, Transport};
use crate::clipboard::{Clipboard, MemoryClipboard};
use crate::coui::{MAIN_PAGE, UiFiles};
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

/// Wait up to `timeout` for a message on `rx`. On macOS the waiting thread is CEF's UI
/// thread too, so it runs CEF's work meanwhile (every [`PUMP_EVERY`]).
fn recv<T>(rx: &mpsc::Receiver<T>, timeout: Duration) -> Option<T> {
	#[cfg(target_os = "macos")]
	{
		let deadline = Instant::now() + timeout;
		loop {
			mac::pump();
			let left = deadline.checked_duration_since(Instant::now()).unwrap_or_default();
			match rx.recv_timeout(left.min(PUMP_EVERY)) {
				Ok(message) => return Some(message),
				Err(mpsc::RecvTimeoutError::Disconnected) => return None,
				Err(mpsc::RecvTimeoutError::Timeout) if left.is_zero() => return None,
				Err(mpsc::RecvTimeoutError::Timeout) => {}
			}
		}
	}
	#[cfg(not(target_os = "macos"))]
	rx.recv_timeout(timeout).ok()
}

/// How often a waiting game thread runs CEF's work on macOS.
#[cfg(target_os = "macos")]
const PUMP_EVERY: Duration = Duration::from_millis(1);

/// What a host says when CEF itself couldn't start (its own log, `cef.log`, says why).
pub const CEF_FAILED: &str = "the browser engine (CEF) failed to start (see its log, cef.log)";

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
	#[cfg(target_os = "macos")]
	if let Err(e) = mac::load_framework(&cef_dir()) {
		eprintln!("bb-ui: {e}");
		return std::env::args().any(|a| a.starts_with("--type=")).then_some(1);
	}
	let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
	let args = Args::new();
	let command_line = args.as_cmd_line()?;
	if command_line.has_switch(Some(&"type".into())) == 0 {
		// SAFETY: first thing in `main` (this function's contract), so no other thread
		// exists yet.
		#[cfg(target_os = "linux")]
		if let Err(e) = unsafe { fonts::hint_fully() } {
			eprintln!("bb-ui: the page's text won't be laid out in whole pixels: {e}");
		}
		#[cfg(target_os = "macos")]
		if let Err(e) = mac::install_application() {
			eprintln!("bb-ui: {e}");
		}
		return None;
	}
	#[cfg(target_os = "macos")]
	if CefString::from(&command_line.switch_value(Some(&"type".into()))).to_string() == "renderer" {
		system_fonts::register();
	}
	let mut app = app::host_app(None, None, None, None, Vec::new());
	Some(execute_process(
		Some(args.as_main_args()),
		Some(&mut app),
		std::ptr::null_mut(),
	))
}

/// The font a generic family names: one of Windows' defaults (on macOS its
/// [`system_fonts`] variant).
fn generic_family(family: &str) -> String {
	#[cfg(target_os = "macos")]
	return system_fonts::generic(family);
	#[cfg(not(target_os = "macos"))]
	family.to_string()
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
	/// A directory CEF may use for its profile and log (created if missing; removed when
	/// the page closes).
	pub cache_dir: PathBuf,
	/// Where CEF writes its log instead of `cef.log` in `cache_dir` (kept: the game's
	/// log folder).
	pub cef_log: Option<PathBuf>,
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
	/// What the page copies to and pastes from: a clipboard of the page's own by
	/// default; the windowed game passes the system's (see [`crate::clipboard`]).
	pub clipboard: Arc<dyn Clipboard>,
	/// Where the page's paints go: the owner's copy of the page's frame, on the GPU
	/// (bb-render's `UiPage`). Required: the page is painted on the GPU only.
	pub sink: Option<Arc<dyn PageSink>>,
	/// The display connection Chromium paints through; `None`: this process's
	/// ([`Display::from_env`]).
	pub display: Option<Display>,
}

impl Default for HostConfig {
	fn default() -> Self {
		HostConfig {
			url: MAIN_PAGE.to_string(),
			width: 1920,
			height: 1080,
			frames: PageFrames::Clock(30),
			cache_dir: std::env::temp_dir().join(format!("bb-ui-{}", std::process::id())),
			cef_log: None,
			devtools_port: None,
			exact_clock: squirrel_bb::ExactClock::Frame,
			battle_camera: squirrel_bb::BattleCamera::Instant,
			battle_motion: squirrel_bb::BattleMotion::Stepped,
			mods: false,
			extra_mods: Vec::new(),
			steam_build: true,
			clipboard: Arc::new(MemoryClipboard::default()),
			sink: None,
			display: None,
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
	/// Counts runs of clicks, which the presses and releases carry.
	clicks: Cell<click_count::ClickCounter<MouseButton>>,
	/// Modifier keys held (`EVENTFLAG_*_DOWN` bits), which every mouse event carries.
	modifiers: Cell<u32>,
	/// When the page's frames begin (see [`PageFrames`]).
	frame_clock: page_frames::FrameClock,
	/// Whether the page gets a turn this frame, or rests ([`page_rest`]).
	rest: page_rest::PageRest,
	cache_dir: PathBuf,
	/// Where CEF was initialized, and is shut down when the host goes.
	cef: Option<cef_thread::CefThread>,
}

impl UiHost {
	/// Start CEF and open the page, serving `coui://` from `files`. Returns at once:
	/// CEF starts on a thread of its own ([`cef_thread`]) and opens the page as soon as
	/// it can ([`opening`]), while the caller goes on with its own start;
	/// [`wait_open`](Self::wait_open) waits for the page's browser.
	pub fn start(files: Arc<UiFiles>, config: HostConfig) -> Result<UiHost, String> {
		let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
		let sink = config
			.sink
			.clone()
			.ok_or("no paint sink: the page is painted on the GPU, and its owner keeps the frame (HostConfig::sink)")?;
		let display = match config.display {
			Some(display) => display,
			None => Display::from_env()?,
		};
		std::fs::create_dir_all(&config.cache_dir).map_err(|e| format!("cache dir: {e}"))?;
		// Bound before CEF starts: the renderer is told where it is as it launches.
		let bridge = bridge_wire::Listener::bind().map_err(|e| format!("the bridge's socket: {e}"))?;
		let bridge_switch = bridge.endpoint().to_switch();

		let (events_tx, events) = mpsc::channel();
		let shared = Arc::new(Shared {
			events: Mutex::new(events_tx),
			browser: Mutex::new(None),
			sink,
			frame: Mutex::new(Default::default()),
			paint_error: Mutex::new(None),
			painted_in_memory: Default::default(),
			size: Mutex::new((config.width as i32, config.height as i32)),
			scale: Mutex::new(1.0),
			images_served: AtomicU64::new(0),
			loads_finished: AtomicU64::new(0),
			paint_counters: Default::default(),
			clipboard: config.clipboard.clone(),
		});
		bridge_server::start(bridge, shared.clone());
		let opening = opening::Opening {
			files,
			shared: shared.clone(),
			url: config.url.clone(),
			external_frames: config.frames.external(),
			frame_rate: config.frames.clock_rate(),
		};
		let (devtools_port, ozone) = (config.devtools_port, display.ozone());
		let switches = config.sink.as_ref().map(|sink| sink.browser_switches()).unwrap_or_default();
		let (cache_dir, cef_log) = (config.cache_dir.clone(), config.cef_log.clone());
		let make = move || {
			let app = app::host_app(Some(opening), devtools_port, Some(bridge_switch), ozone, switches);
			let path = |p: PathBuf| CefString::from(p.to_string_lossy().as_ref());
			let cef_dir = cef_dir();
			let settings = Settings {
				no_sandbox: 1,
				windowless_rendering_enabled: 1,
				root_cache_path: path(cache_dir.clone()),
				log_file: path(cef_log.unwrap_or_else(|| cache_dir.join("cef.log"))),
				log_severity: LogSeverity::WARNING,
				// Transparent: the UI is drawn over the game's scene.
				background_color: 0,
				remote_debugging_port: devtools_port.map_or(0, i32::from),
				..Default::default()
			};
			// CEF's UI thread: its own on Windows and Linux; on macOS the main thread,
			// which runs CEF's work whenever the game polls or waits (`mac`). There the
			// framework holds the resources, and every child process runs this program too.
			#[cfg(not(target_os = "macos"))]
			let settings = Settings {
				multi_threaded_message_loop: 1,
				resources_dir_path: path(cef_dir.clone()),
				locales_dir_path: path(cef_dir.join("locales")),
				..settings
			};
			#[cfg(target_os = "macos")]
			let settings = Settings {
				external_message_pump: 1,
				framework_dir_path: mac::framework_dir(&cef_dir).map(path).unwrap_or_default(),
				browser_subprocess_path: mac::subprocess_path().map(path).unwrap_or_default(),
				main_bundle_path: mac::main_bundle().map(path).unwrap_or_default(),
				..settings
			};
			(app, settings)
		};
		let failing = shared.clone();
		let failed = move || failing.send(Event::OpenFailed(CEF_FAILED.into()));
		let cef = cef_thread::CefThread::spawn(make, failed)?;
		Ok(UiHost {
			shared,
			events,
			deferred: RefCell::new(VecDeque::new()),
			pending: RefCell::new(None),
			parked: Cell::new(false),
			notes: RefCell::new(Vec::new()),
			next_request: Cell::new(1),
			pointer: Cell::new(None),
			held: Cell::new(0),
			clicks: Cell::new(click_count::ClickCounter::new(
				click_count::DoubleClick::system(),
			)),
			modifiers: Cell::new(0),
			frame_clock: page_frames::FrameClock::new(config.frames),
			rest: page_rest::PageRest::new(page_rest::PageRest::wanted(
				config.frames.external(),
				config.devtools_port.is_some(),
			)),
			cache_dir: config.cache_dir,
			cef: Some(cef),
		})
	}

	/// Wait until the page's browser exists (it can take messages), for up to `timeout`.
	pub fn wait_open(&self, timeout: Duration) -> Result<(), String> {
		if self.shared.browser.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
			return Ok(());
		}
		match self.wait_for(timeout, |e| matches!(e, Event::Created | Event::OpenFailed(_))) {
			Some(Event::Created) => {
				squirrel_bb::startup::mark("browser created");
				Ok(())
			}
			Some(Event::OpenFailed(why)) => Err(why),
			_ => Err(format!("{CEF_FAILED}: the page's browser didn't open within {timeout:?}")),
		}
	}

	/// Let `duration` pass. On macOS the waiting thread is CEF's UI thread, so it runs
	/// CEF's work meanwhile (as [`recv`] does): a loop that slept between frames held up
	/// the page's work (its frames, paints and calls) until the next frame there.
	/// Elsewhere it sleeps.
	pub fn idle(&self, duration: Duration) {
		#[cfg(target_os = "macos")]
		{
			let deadline = Instant::now() + duration;
			loop {
				mac::pump();
				let left = deadline.saturating_duration_since(Instant::now());
				if left.is_zero() {
					break;
				}
				std::thread::sleep(left.min(PUMP_EVERY));
			}
		}
		#[cfg(not(target_os = "macos"))]
		std::thread::sleep(duration);
	}

	/// When CEF wants its work run next, if it asked: on macOS, where the game runs it
	/// ([`pump`](Self::pump)); never elsewhere, where CEF has a thread of its own.
	pub fn pump_due(&self) -> Option<Instant> {
		pump_schedule::SCHEDULE.due()
	}

	/// Run CEF's work if it is due (macOS; nothing elsewhere).
	pub fn pump(&self) {
		#[cfg(target_os = "macos")]
		if pump_schedule::SCHEDULE.take_due(Instant::now()) {
			mac::pump();
		}
	}

	/// Have `waker` wake the game's loop when CEF wants its work run sooner than it last
	/// asked (macOS; called from any thread). `None` stops it.
	pub fn set_pump_waker(&self, waker: Option<Waker>) {
		pump_schedule::SCHEDULE.set_waker(waker);
	}

	/// Every event that arrived since the last poll, in order. Doesn't wait.
	pub fn poll(&self) -> Vec<Event> {
		#[cfg(target_os = "macos")]
		mac::pump();
		let mut events: Vec<Event> = self.deferred.borrow_mut().drain(..).collect();
		events.extend(self.events.try_iter());
		events
	}

	/// The next event, waiting up to `timeout` for one.
	pub fn next_event(&self, timeout: Duration) -> Option<Event> {
		if let Some(event) = self.deferred.borrow_mut().pop_front() {
			return Some(event);
		}
		recv(&self.events, timeout)
	}

	/// Wait up to `timeout` for an event matching `wanted` and return it; every other
	/// event is kept for the next [`poll`](Self::poll).
	pub fn wait_for(&self, timeout: Duration, wanted: impl Fn(&Event) -> bool) -> Option<Event> {
		let deadline = Instant::now() + timeout;
		loop {
			let left = deadline.checked_duration_since(Instant::now())?;
			let event = recv(&self.events, left)?;
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
			match recv(&self.events, left).ok_or_else(late)? {
				Event::Request {
					kind: RequestKind::Park,
					responder,
					body,
				} => {
					self.rest.reported_text(&body, Instant::now());
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
		self.rest.ran();
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

	/// At the end of a game frame's step, the page parked: let it go on until the next
	/// frame parks it if it has work, or leave it resting, parked across the frames with
	/// none begun until it has work again ([`page_rest`]: what counts as work). Returns
	/// whether it goes on. A page that isn't parked (still loading) goes on.
	pub fn let_page_run(&self) -> bool {
		let paints = self.paints();
		let loads = self.shared.loads_finished.load(Ordering::Relaxed);
		let turn = !self.parked.get() || self.rest.decide(Instant::now(), paints, loads);
		if turn {
			self.unpark();
		}
		turn
	}

	/// Whether the page rests (see [`let_page_run`](Self::let_page_run)).
	pub fn is_resting(&self) -> bool {
		self.rest.resting()
	}

	/// How many keydown and keyup events the page had seen when it last parked (or
	/// answered a call), if it said (`page_work.js`).
	pub fn keys_seen(&self) -> Option<i64> {
		self.rest.keys_seen()
	}

	/// How many game frames so far the page rested through.
	pub fn rested_frames(&self) -> u64 {
		self.rest.rests()
	}

	/// The game gave the page work it must see promptly (a click, the wheel): a resting
	/// page goes on now, as it would have between frames had it not rested. Other work
	/// (a move, a call from the scripts) waits for the next frame's turn.
	fn wake_now(&self) {
		self.rest.wake();
		if self.rest.resting() {
			self.unpark();
		}
	}

	/// The game discarded a scene (a battle's end, the campaign left, a save loaded):
	/// the page gives back what it kept for it ([`memory`]).
	pub fn release_memory(&self) {
		// The renderer's part runs on its main thread.
		self.rest.wake();
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
			// A page that isn't waiting isn't resting either; what the call changed
			// may need its next frame.
			self.rest.wake();
			self.send(kind, &[Arg::Int(id), Arg::Str(text)])?;
		}

		let mut deadline = Instant::now() + CALL_TIMEOUT;
		loop {
			let left = deadline
				.checked_duration_since(Instant::now())
				.ok_or_else(|| format!("the page did not answer within {CALL_TIMEOUT:?}"))?;
			let event = recv(&self.events, left).ok_or_else(|| format!("the page did not answer within {CALL_TIMEOUT:?}"))?;
			match event {
				Event::Reply { id: reply, result } if !nested && reply == id => return result,
				Event::Request {
					kind: RequestKind::Resume,
					body,
					responder,
				} if nested => {
					self.pending.replace(Some(responder));
					// What the page has to do after the call (whether it may go on
					// resting), a newline, then the call's result.
					let (work, result) = body.split_once('\n').unwrap_or(("", body.as_str()));
					self.rest.called(work, Instant::now());
					return Ok(result.to_string());
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
		self.rest.wake();
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

	/// The modifier keys held now. Every mouse event carries them from here on, as a
	/// player's do: the page reads them as `ctrlKey`, `altKey` and `shiftKey` (the
	/// inventory's Ctrl- and Alt-clicks, the shop's Alt+right-click repair).
	pub fn set_modifiers(&self, modifiers: Modifiers) {
		self.modifiers.set(modifiers.flags());
	}

	/// What a mouse event carries: the buttons and modifier keys held.
	fn mouse_flags(&self) -> u32 {
		self.held.get() | self.modifiers.get()
	}

	/// Move the mouse to `(x, y)` (view pixels).
	pub fn mouse_move(&self, x: i32, y: i32) {
		self.rest.wake();
		if let Some(host) = self.browser_host() {
			self.pointer.set(Some((x, y)));
			self.update_clicks(|c| c.moved((x, y)));
			let modifiers = self.mouse_flags();
			host.send_mouse_move_event(Some(&MouseEvent { x, y, modifiers }), 0);
		}
	}

	/// Move the mouse out of the view, if it's in it (hover ends).
	pub fn mouse_leave(&self) {
		self.rest.wake();
		if let (Some((x, y)), Some(host)) = (self.pointer.take(), self.browser_host()) {
			self.update_clicks(|c| c.reset());
			let modifiers = self.mouse_flags();
			host.send_mouse_move_event(Some(&MouseEvent { x, y, modifiers }), 1);
		}
	}

	/// Left-click at `(x, y)`: move there, press, release. A click soon after another
	/// in the same place continues its run (a double click, a triple click), as a
	/// player's would.
	pub fn click(&self, x: i32, y: i32) {
		self.wake_now();
		let Some(host) = self.browser_host() else {
			return;
		};
		self.pointer.set(Some((x, y)));
		self.update_clicks(|c| c.moved((x, y)));
		let count = self.update_clicks(|c| c.press(MouseButton::Left, (x, y), Instant::now()));
		let event = MouseEvent { x, y, modifiers: self.modifiers.get() };
		host.send_mouse_move_event(Some(&event), 0);
		host.send_mouse_click_event(Some(&event), MouseButtonType::LEFT, 0, count);
		host.send_mouse_click_event(Some(&event), MouseButtonType::LEFT, 1, count);
	}

	/// Double-click at `(x, y)`: two clicks there, the first starting a new run.
	pub fn double_click(&self, x: i32, y: i32) {
		self.update_clicks(|c| c.reset());
		self.click(x, y);
		self.click(x, y);
	}

	/// Run `f` on the click counter, and give back what it returns.
	fn update_clicks<R>(&self, f: impl FnOnce(&mut click_count::ClickCounter<MouseButton>) -> R) -> R {
		let mut clicks = self.clicks.get();
		let result = f(&mut clicks);
		self.clicks.set(clicks);
		result
	}

	/// `button` went down or up at `(x, y)` (view pixels), as a player's mouse does it.
	/// Moves carry the buttons held, so the page sees drags.
	pub fn mouse_button(&self, x: i32, y: i32, button: MouseButton, down: bool) {
		self.wake_now();
		let Some(host) = self.browser_host() else {
			return;
		};
		self.pointer.set(Some((x, y)));
		let count = self.update_clicks(|c| {
			c.moved((x, y));
			if down {
				c.press(button, (x, y), Instant::now())
			} else {
				c.release(button)
			}
		});
		let flag = button.held_flag();
		let held = if down {
			self.held.get() | flag
		} else {
			self.held.get() & !flag
		};
		// The event itself carries the button while it goes down and until it is up.
		live_input::button(&host, x, y, button, down, self.mouse_flags() | flag, count);
		self.held.set(held);
	}

	/// Key `code` (a Windows virtual-key code) went down or up in the focused element.
	/// The page sees it in the game's hand-off of a frame's keys, which unparks it
	/// (`UiSession::keys_to_page`): it gets no turn after it for that alone; what its
	/// listeners do gives it one, as the page says when it parks (`page_work.js`).
	pub fn key(&self, code: i32, down: bool, modifiers: Modifiers) {
		if let Some(host) = self.browser_host() {
			live_input::key(&host, code, down, modifiers);
		}
	}

	/// The keys typed `c` in the focused element (after the key went down); as with
	/// [`key`](Self::key), the hand-off is the page's turn for it.
	pub fn character(&self, c: char, modifiers: Modifiers) {
		if let Some(host) = self.browser_host() {
			live_input::character(&host, c, modifiers);
		}
	}

	/// Press and release `key` in the focused element.
	pub fn press_key(&self, key: Key) {
		self.wake_now();
		if let Some(host) = self.browser_host() {
			input::press_key(&host, key);
		}
	}

	/// Type `text` into the focused element, a key press per character.
	pub fn type_text(&self, text: &str) {
		self.wake_now();
		if let Some(host) = self.browser_host() {
			input::type_text(&host, text);
		}
	}

	/// Turn the mouse wheel `notches` times at `(x, y)`; positive scrolls up.
	pub fn wheel(&self, x: i32, y: i32, notches: i32) {
		self.wake_now();
		if let Some(host) = self.browser_host() {
			self.pointer.set(Some((x, y)));
			self.update_clicks(|c| c.moved((x, y)));
			input::wheel(&host, x, y, notches, self.mouse_flags());
		}
	}

	/// A copy of the latest painted frame, read back from the GPU (a whole page's worth:
	/// for screenshots and checks, not every frame). `None` before the first paint.
	pub fn frame(&self) -> Option<FrameBuffer> {
		let paints = self.paints();
		let (width, height) = self.painted_size()?;
		match self.shared.sink.read(PaintRect::whole(width, height)) {
			Ok(Some((region, bgra))) => Some(FrameBuffer { width: region.width, height: region.height, bgra, paints }),
			Ok(None) => None,
			Err(e) => {
				eprintln!("bb-ui: could not read the page's frame back: {e}");
				None
			}
		}
	}

	/// The latest paint's size, if there was one.
	fn painted_size(&self) -> Option<(u32, u32)> {
		let state = self.shared.frame.lock().unwrap_or_else(|e| e.into_inner());
		(state.paints > 0).then_some((state.width, state.height))
	}

	/// The painted page's alpha at `(x, y)` (painted pixels: the screen's, see
	/// [`UiHost::resize`]): 0 outside it, or before the first paint. Read back from the
	/// GPU a tile at a time, kept until a paint changes the tile ([`frame`]).
	pub fn alpha_at(&self, x: i32, y: i32) -> u8 {
		let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
			return 0;
		};
		let (tile, paints) = {
			let state = self.shared.frame.lock().unwrap_or_else(|e| e.into_inner());
			if state.paints == 0 || x >= state.width || y >= state.height {
				return 0;
			}
			if let Some(alpha) = state.alpha(x, y) {
				return alpha;
			}
			(state.tile_at(x, y), state.paints)
		};
		let bgra = match self.shared.sink.read(tile) {
			Ok(Some((region, bgra))) if region == tile => bgra,
			Ok(_) => return 0,
			Err(e) => {
				eprintln!("bb-ui: could not read the page's alpha back: {e}");
				return 0;
			}
		};
		let alpha = bgra[(((y - tile.y) * tile.width + (x - tile.x)) * 4 + 3) as usize];
		let mut state = self.shared.frame.lock().unwrap_or_else(|e| e.into_inner());
		// A paint landed while the tile was read: it may have read either side of it.
		if state.paints == paints {
			state.keep(tile, &bgra);
		}
		alpha
	}

	/// How many frames have been painted so far.
	pub fn paints(&self) -> u64 {
		self.shared.frame.lock().unwrap_or_else(|e| e.into_inner()).paints
	}

	/// Whether Chromium painted the page in memory: its GPU compositing failed, so it
	/// won't paint on the GPU (where the page is shown) either.
	pub fn painted_in_memory(&self) -> bool {
		self.shared.painted_in_memory.load(std::sync::atomic::Ordering::Relaxed)
	}

	/// Why the last paint the page's sink couldn't take couldn't, if one couldn't.
	pub fn paint_error(&self) -> Option<String> {
		self.shared.paint_error.lock().unwrap_or_else(|e| e.into_inner()).clone()
	}

	/// The game drew a frame: with [`PageFrames::Game`], the page makes its next frame
	/// now (and paints what changed by the next game frame). Nothing to do on Chromium's
	/// clock.
	pub fn begin_frame(&self) {
		// A resting page has nothing to make a frame of (and couldn't: it's parked).
		if !self.frame_clock.external() || self.rest.resting() {
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
		pump_schedule::SCHEDULE.set_waker(None);
		// Release a page still waiting on the game.
		drop(self.pending.take());
		let browser = || self.shared.browser.lock().unwrap_or_else(|e| e.into_inner()).clone();
		// A page still being opened is let open first, so it can be closed.
		let browser = browser().or_else(|| self.wait_open(Duration::from_secs(10)).ok().and_then(|()| browser()));
		if let Some(host) = browser.and_then(|b| b.host()) {
			host.close_browser(1);
			self.wait_for(Duration::from_secs(10), |e| matches!(e, Event::Closed));
		}
		// CEF shuts down where it was initialized.
		drop(self.cef.take());
		let _ = std::fs::remove_dir_all(&self.cache_dir);
	}
}
