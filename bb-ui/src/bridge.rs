//! The game-thread end of the UI bridge: speaks the Coherent protocol that the game's
//! JS (`ui/screens.js`, `ui/coherent.js`) expects, over any [`Transport`].
//!
//! **Everything runs inline, as in the original** (probed 2026-10-13): an SQ→JS call
//! returns only after the JS has run, and a JS→SQ call returns only after the script
//! has run. Calls nest both ways: JS run by `asyncCall` may call a script, which may
//! call JS again, and so on.
//!
//! **SQ → JS.** As a [`UiFrontend`] of the script-side [`UiBridge`]:
//! - `UI.connect` / `connectToModule` run `callFromSQ(screen, module or "",
//!   "onConnection", handle)`. The JS screen stores the handle (an integer) and passes
//!   it back on every call.
//! - `UI.disconnect` runs `callFromSQNoArgs(screen, module or "", "onDisconnection")`;
//!   for a module it then runs the screen's `onModuleOnDisconnectionCalled`, with no
//!   argument.
//! - `JSHandle.asyncCall(method, data)` and `JSHandle.call(method, data)` (the same
//!   native in the original) run `callFromSQ(screen, module or "", method, data)`.
//!
//! **JS → SQ.** Every `engine.call(name, ...)` arrives as `[name, requestId, args...]`
//! and gets a [`Reply`] ([`handle_message`](UiProtocol::handle_message)):
//! - `registerScreen(name)`; `registrationFinished()`, after which the engine itself
//!   connects `RootScreen` (with a `null` handle; every other screen's container is
//!   created inside it) and sets its version label;
//! - `callSQ(handle, method[, arg])`: `obj.method([arg])`; its return value resolves the
//!   page's promise;
//! - `asyncCallSQ(handle, method[, arg])`: the same, but the original never resolves the
//!   promise, nor those of the other void calls;
//! - `openURL(url)`: opened in the system's browser, as the original does, by the
//!   opener the game installs ([`set_url_opener`](UiProtocol::set_url_opener)); without
//!   one (tests, examples) only logged. Only `http(s)` URLs are opened ([`web_url`]);
//! - a name starting with [`ENGINE_PREFIX`] (`bb.`): the engine's own, for what it adds
//!   to the page beside the game's UI (its update notice, `bb.updateNotice(choice)`).
//!   Neither the game's JS nor its mods' call such names (Coherent had none): each goes
//!   to the handler the game set for it ([`set_engine_handler`](UiProtocol::set_engine_handler)),
//!   inline on the game's thread like the others, and is an unknown handler without one.
//!
//! **Timing.** With `BB_UI_SLOW_CALLS=<ms>` in the environment, every call either way
//! that takes at least that long is logged to stderr with its time, for profiling the
//! game's frames (`0` logs them all). A call's time includes the calls it caused.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use squirrel::{SQVM, SqRef, SqStrongRef, SqTable};
use squirrel_bb::{UiBridge, UiFrontend};

use crate::json::Json;
use crate::sq_json::{from_json, strong_to_json, to_json};
use crate::web_url;

/// The date the original's root version label shows next to the version: the build
/// date of the 1.5.2.3 executable (probed).
const BUILD_DATE: &str = "2026-06-22 15:00:00";

/// The environment variable that turns on logging slow calls (see the module docs).
const SLOW_CALLS: &str = "BB_UI_SLOW_CALLS";

/// What the names of the engine's own `engine.call` handlers start with (see the
/// module docs).
pub const ENGINE_PREFIX: &str = "bb.";

/// Coherent's error code for a call to a handler that doesn't exist
/// (`engine._Errors[2]`).
const NO_SUCH_METHOD: i64 = 2;

/// What the game answers to one `engine.call` from the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
	/// No answer at all: the original never resolves `registerScreen`,
	/// `registrationFinished`, `asyncCallSQ` or `openURL` (their promises stay pending).
	Void,
	/// `engine._Result(id, value)`: what a `callSQ` script method returned.
	Value(Json),
	/// `engine._Result(id)` with no value: the `callSQ` script method raised.
	NoValue,
	/// `engine._OnError(id, [NoSuchMethod])`: no such handler.
	Error(String),
}

impl Reply {
	/// The reply as the page's bridge script reads it.
	pub fn to_json(&self) -> Json {
		let mut entries = vec![("done".to_string(), Json::Bool(true))];
		match self {
			Reply::Void => {}
			Reply::Value(value) => entries.push(("result".into(), value.clone())),
			Reply::NoValue => entries.push(("noValue".into(), Json::Bool(true))),
			Reply::Error(message) => entries.push((
				"errors".into(),
				Json::Array(vec![Json::Object(vec![
					("first".into(), Json::Int(NO_SUCH_METHOD)),
					("second".into(), Json::Str(message.clone())),
				])]),
			)),
		}
		Json::Object(entries)
	}
}

/// The browser side of the bridge, as the game thread sees it.
pub trait Transport {
	/// Run `entry` (`["t", event, args...]`: `engine._trigger(event, args...)`) in the
	/// page and return once it has finished. Every `engine.call` the page makes
	/// meanwhile is passed to `serve`, which runs it in the game (possibly calling back
	/// into the page, re-entering `run`) and returns the reply.
	fn run(&self, entry: &Json, serve: &mut dyn FnMut(&Json) -> Reply) -> Result<(), String>;
}

/// The protocol state on the game thread. Attach it to the [`UiBridge`] with
/// [`attach`](Self::attach) and hand it the page's spontaneous calls (user input) with
/// [`handle_message`](Self::handle_message).
pub struct UiProtocol<T: Transport> {
	transport: T,
	screens: RefCell<Vec<String>>,
	registration_finished: Cell<bool>,
	stats: Cell<CallStats>,
	/// Calls this slow are logged ([`SLOW_CALLS`]).
	slow_calls: Option<Duration>,
	log: Box<dyn Fn(&str)>,
	/// What opens the page's `openURL` URLs (already checked), if anything does.
	url_opener: RefCell<Option<Box<UrlOpener>>>,
	/// The engine's own handlers, by name (each starts with [`ENGINE_PREFIX`]).
	engine_handlers: RefCell<Vec<(String, Rc<EngineHandler>)>>,
}

/// One of the engine's own `engine.call` handlers (see
/// [`UiProtocol::set_engine_handler`]): given the call's arguments, its reply.
pub type EngineHandler = dyn Fn(&[Json]) -> Reply;

/// Opens a URL in the system's browser (see [`UiProtocol::set_url_opener`]); an error
/// says why it couldn't.
pub type UrlOpener = dyn Fn(&str) -> Result<(), String>;

/// The game's calls into the page so far: how many, and how long they took, counting
/// the page's calls back into the game that they caused.
#[derive(Clone, Copy, Debug, Default)]
pub struct CallStats {
	pub calls: u64,
	pub time: Duration,
}

impl<T: Transport + 'static> UiProtocol<T> {
	/// A protocol over `transport`; `log` receives script errors and protocol warnings.
	pub fn new(transport: T, log: impl Fn(&str) + 'static) -> Rc<Self> {
		Rc::new(UiProtocol {
			transport,
			screens: RefCell::new(Vec::new()),
			registration_finished: Cell::new(false),
			stats: Cell::new(CallStats::default()),
			slow_calls: std::env::var(SLOW_CALLS)
				.ok()
				.and_then(|ms| ms.trim().parse::<f64>().ok())
				.map(|ms| Duration::from_secs_f64(ms.max(0.0) / 1000.0)),
			log: Box::new(log),
			url_opener: RefCell::new(None),
			engine_handlers: RefCell::new(Vec::new()),
		})
	}

	/// Route the bridge's SQ→JS calls through this protocol.
	pub fn attach(self: &Rc<Self>, bridge: &UiBridge) {
		bridge.set_frontend(self.clone());
	}
}

impl<T: Transport> UiProtocol<T> {
	/// The transport (e.g. for host-specific calls such as evaluating test JS).
	pub fn transport(&self) -> &T {
		&self.transport
	}

	/// Whether the page has registered all its screens (`registrationFinished`).
	pub fn is_registered(&self) -> bool {
		self.registration_finished.get()
	}

	/// The screens the page registered, in order.
	pub fn screens(&self) -> Vec<String> {
		self.screens.borrow().clone()
	}

	/// The game's calls into the page so far.
	pub fn stats(&self) -> CallStats {
		self.stats.get()
	}

	/// Open the page's `openURL` URLs with `opener` (the game passes one that starts the
	/// system's browser). It gets only those [`web_url::to_open`] passes, as that makes
	/// them, and must not block. Without one, they are only logged.
	pub fn set_url_opener(&self, opener: impl Fn(&str) -> Result<(), String> + 'static) {
		*self.url_opener.borrow_mut() = Some(Box::new(opener));
	}

	/// Answer the page's `engine.call(name, ...)` with `handler`, replacing any it had:
	/// one of the engine's own calls, so `name` must start with [`ENGINE_PREFIX`]. It runs
	/// on the game's thread, inside the page's call, and must not call into the page.
	pub fn set_engine_handler(&self, name: &str, handler: impl Fn(&[Json]) -> Reply + 'static) {
		assert!(name.starts_with(ENGINE_PREFIX), "the engine's own handlers start with {ENGINE_PREFIX}: {name}");
		let mut handlers = self.engine_handlers.borrow_mut();
		handlers.retain(|(n, _)| n != name);
		handlers.push((name.to_string(), Rc::new(handler)));
	}

	/// Run `engine._trigger(event, args...)` in the page now, serving the page's calls
	/// back into the game while it runs.
	pub fn trigger(
		&self,
		vm: &SQVM,
		bridge: &UiBridge,
		event: &str,
		args: impl IntoIterator<Item = Json>,
	) {
		let mut entry = vec![Json::Str("t".into()), Json::Str(event.into())];
		entry.extend(args);
		let entry = Json::Array(entry);
		let mut serve = |message: &Json| self.handle_message(vm, bridge, message);
		let start = Instant::now();
		let span = squirrel_bb::frame_timing::span("bridge game->page");
		squirrel_bb::frame_timing::count("bridge game->page", 1);
		let result = self.transport.run(&entry, &mut serve);
		drop(span);
		let mut stats = self.stats.get();
		stats.calls += 1;
		stats.time += start.elapsed();
		self.stats.set(stats);
		self.note_time("game → page", start, || describe(&entry));
		if let Err(error) = result {
			(self.log)(&format!(
				"ui: {} failed in the page: {error}",
				describe(&entry)
			));
		}
	}

	/// Handle one `engine.call` from the page: `[name, requestId, args...]`.
	pub fn handle_message(&self, vm: &SQVM, bridge: &UiBridge, message: &Json) -> Reply {
		let start = Instant::now();
		let _span = squirrel_bb::frame_timing::span("bridge page->game");
		squirrel_bb::frame_timing::count("bridge page->game", 1);
		let reply = self.dispatch(vm, bridge, message);
		self.note_time("page → game", start, || describe_message(message));
		reply
	}

	/// Log a call that started at `start` if it was slow ([`SLOW_CALLS`]).
	fn note_time(&self, direction: &str, start: Instant, what: impl FnOnce() -> String) {
		let Some(threshold) = self.slow_calls else {
			return;
		};
		let time = start.elapsed();
		if time >= threshold {
			eprintln!(
				"bb-ui: {direction} {:.3} ms {}",
				time.as_secs_f64() * 1000.0,
				what()
			);
		}
	}

	/// [`handle_message`](Self::handle_message), untimed.
	fn dispatch(&self, vm: &SQVM, bridge: &UiBridge, message: &Json) -> Reply {
		let Some(items) = message.as_array().filter(|items| items.len() >= 2) else {
			(self.log)(&format!(
				"ui: malformed message from the page: {}",
				message.to_text()
			));
			return Reply::Void;
		};
		let (name, args) = (items[0].as_str().unwrap_or(""), &items[2..]);
		match name {
			"registerScreen" => {
				if let Some(screen) = args.first().and_then(Json::as_str) {
					self.screens.borrow_mut().push(screen.to_string());
				}
				Reply::Void
			}
			"registrationFinished" => {
				self.registration_finished.set(true);
				self.connect_root_screen(vm, bridge);
				Reply::Void
			}
			"callSQ" => self.call_script(vm, bridge, args),
			"asyncCallSQ" => {
				self.call_script(vm, bridge, args);
				Reply::Void
			}
			"openURL" => {
				self.open_url(args.first());
				Reply::Void
			}
			_ if name.starts_with(ENGINE_PREFIX) && let Some(handler) = self.engine_handler(name) => handler(args),
			_ => {
				(self.log)(&format!("ui: engine.call to unknown handler '{name}'"));
				Reply::Error(format!("no handler '{name}'"))
			}
		}
	}

	/// The engine's own handler named `name`, if the game set one (cloned out, so the
	/// handler may set handlers itself).
	fn engine_handler(&self, name: &str) -> Option<Rc<EngineHandler>> {
		self.engine_handlers.borrow().iter().find(|(n, _)| n == name).map(|(_, h)| h.clone())
	}

	/// `openURL(url)`: open `url` with the opener, if it may be opened and there is one.
	/// Whatever happens is logged (the URL as JSON, so nothing in it reaches the log raw).
	fn open_url(&self, url: Option<&Json>) {
		let shown = url.map_or(String::new(), Json::to_text);
		let outcome = match url.and_then(Json::as_str).ok_or("not text").and_then(web_url::to_open) {
			Err(why) => format!("refused: {why}"),
			Ok(url) => match self.url_opener.borrow().as_ref() {
				None => "not opened".to_string(),
				Some(open) => match open(&url) {
					Ok(()) => "opened".to_string(),
					Err(error) => format!("not opened: {error}"),
				},
			},
		};
		(self.log)(&format!("ui: openURL {shown} ({outcome})"));
	}

	/// What the engine does once the page has registered its screens: connect
	/// `RootScreen` (it has no script object, so its handle is `null`) and show the
	/// version in its version module.
	fn connect_root_screen(&self, vm: &SQVM, bridge: &UiBridge) {
		let root = || Json::Str("RootScreen".into());
		self.trigger(
			vm,
			bridge,
			"callFromSQ",
			[
				root(),
				Json::Str(String::new()),
				Json::Str("onConnection".into()),
				Json::Null,
			],
		);
		let label = format!("Build: {} - Date: {BUILD_DATE}", version(vm));
		self.trigger(
			vm,
			bridge,
			"callFromSQ",
			[
				root(),
				Json::Str("VersionModule".into()),
				Json::Str("setVersion".into()),
				Json::Str(label),
			],
		);
	}

	/// `obj.method([arg])` on the script object behind handle `args[0]`.
	fn call_script(&self, vm: &SQVM, bridge: &UiBridge, args: &[Json]) -> Reply {
		let (Some(handle), Some(method)) = (args.first(), args.get(1).and_then(Json::as_str))
		else {
			(self.log)("ui: callSQ without a handle and method");
			return Reply::Value(Json::Null);
		};
		let Some(receiver) = handle
			.as_i64()
			.and_then(|h| u64::try_from(h).ok())
			.and_then(|h| bridge.receiver_for_handle(h))
		else {
			(self.log)(&format!(
				"ui: callSQ '{method}' on a stale handle {}",
				handle.to_text()
			));
			return Reply::Value(Json::Null);
		};
		// `undefined` (Coherent forwards it from wrappers that pass extra arguments) and
		// `null` alike mean "no argument".
		let result = match args.get(2) {
			Some(data) => receiver.call_method_ret::<SqRef, _>(vm, method, (from_json(vm, data),)),
			None => receiver.call_method_ret::<SqRef, _>(vm, method, ()),
		};
		match result {
			Ok(value) => Reply::Value(value.map_or(Json::Null, |v| to_json(&v))),
			Err(error) if error.starts_with("no method") => Reply::Value(Json::Null),
			// The VM's error handler has reported it (a `Script Error` row with its
			// stack), the original's only trace of it: no line of the bridge's own
			// (probed 2026-10-21). The page's callback runs with nothing.
			Err(_) => Reply::NoValue,
		}
	}
}

/// `GameInfo.getVersionNumber() + " " + GameInfo.getVersionName()`, as the scripts
/// see them.
fn version(vm: &SQVM) -> String {
	let Some(info) = SqTable::root(vm).get::<SqStrongRef>("GameInfo") else {
		return String::new();
	};
	let part = |name: &str| {
		info.call_method_ret::<String, _>(vm, name, ())
			.ok()
			.flatten()
			.unwrap_or_default()
	};
	format!("{} {}", part("getVersionNumber"), part("getVersionName"))
}

/// `screen.module.method` of an entry, for messages.
fn describe(entry: &Json) -> String {
	let items = entry.as_array().unwrap_or_default();
	items
		.iter()
		.skip(2)
		.take(3)
		.filter_map(Json::as_str)
		.filter(|s| !s.is_empty())
		.collect::<Vec<_>>()
		.join(".")
}

/// `name method` of a message from the page (`[name, requestId, handle, method, ...]`
/// for `callSQ`), for messages.
fn describe_message(message: &Json) -> String {
	let items = message.as_array().unwrap_or_default();
	let name = items.first().and_then(Json::as_str).unwrap_or("?");
	match items.get(3).and_then(Json::as_str) {
		Some(method) => format!("{name} {method}"),
		None => name.to_string(),
	}
}

/// The module argument of `callFromSQ`: the original sends `""` for a screen.
fn module_arg(module: Option<&str>) -> Json {
	Json::Str(module.unwrap_or("").into())
}

impl<T: Transport> UiFrontend for UiProtocol<T> {
	fn connect(
		&self,
		vm: &SQVM,
		bridge: &UiBridge,
		handle: u64,
		screen: &str,
		module: Option<&str>,
	) {
		self.trigger(
			vm,
			bridge,
			"callFromSQ",
			[
				Json::Str(screen.into()),
				module_arg(module),
				Json::Str("onConnection".into()),
				Json::Int(handle as i64),
			],
		);
	}

	fn disconnect(
		&self,
		vm: &SQVM,
		bridge: &UiBridge,
		_handle: u64,
		screen: &str,
		module: Option<&str>,
	) {
		self.trigger(
			vm,
			bridge,
			"callFromSQNoArgs",
			[
				Json::Str(screen.into()),
				module_arg(module),
				Json::Str("onDisconnection".into()),
			],
		);
		if module.is_some() {
			self.trigger(
				vm,
				bridge,
				"callFromSQNoArgs",
				[
					Json::Str(screen.into()),
					Json::Str(String::new()),
					Json::Str("onModuleOnDisconnectionCalled".into()),
				],
			);
		}
	}

	fn call(
		&self,
		vm: &SQVM,
		bridge: &UiBridge,
		screen: &str,
		module: Option<&str>,
		method: &str,
		data: &SqStrongRef,
	) {
		let mut args = vec![
			Json::Str(screen.into()),
			module_arg(module),
			Json::Str(method.into()),
		];
		// Data JS can't hold arrives as `undefined`: leave the argument out.
		args.extend(strong_to_json(vm, data));
		self.trigger(vm, bridge, "callFromSQ", args);
	}
}

#[cfg(test)]
mod tests {
	use std::cell::RefCell;
	use std::rc::Rc;

	use squirrel::SQVM;
	use squirrel_bb::UiBridge;

	use super::{Reply, Transport, UiProtocol};
	use crate::json::Json;

	/// A stand-in page: records every entry it is asked to run, and answers each
	/// `onConnection` the way the vanilla screens do, by calling the script's
	/// `onScreenConnected` back through the handle it was given.
	#[derive(Default, Clone)]
	struct FakePage {
		ran: Rc<RefCell<Vec<String>>>,
		replies: Rc<RefCell<Vec<Reply>>>,
	}

	impl Transport for FakePage {
		fn run(&self, entry: &Json, serve: &mut dyn FnMut(&Json) -> Reply) -> Result<(), String> {
			self.ran.borrow_mut().push(entry.to_text());
			let items = entry.as_array().unwrap();
			if items[4] == Json::Str("onConnection".into()) && items[5] != Json::Null {
				let call = Json::Array(vec![
					Json::Str("asyncCallSQ".into()),
					Json::Int(99),
					items[5].clone(),
					Json::Str("onScreenConnected".into()),
				]);
				let reply = serve(&call);
				self.replies.borrow_mut().push(reply);
			}
			Ok(())
		}
	}

	fn setup() -> (SQVM, UiBridge, FakePage, Rc<UiProtocol<FakePage>>) {
		let mut vm = SQVM::new(1024);
		let bridge = UiBridge::new();
		bridge.install(&mut vm);
		let page = FakePage::default();
		let protocol = UiProtocol::new(page.clone(), |m| panic!("unexpected log: {m}"));
		protocol.attach(&bridge);
		(vm, bridge, page, protocol)
	}

	fn message(text: &str) -> Json {
		Json::parse(text).unwrap()
	}

	#[test]
	fn connecting_runs_the_page_inline() {
		let (vm, bridge, page, _protocol) = setup();
		vm.run_script(
			r#"
			::screen <- { connected = false, function onScreenConnected() { this.connected = true; } };
			::h <- ::UI.connect("MainMenuScreen", ::screen);
			// The page answered before UI.connect returned.
			assert(::screen.connected);
			::m <- ::h.connectToModule("MainMenuModule", {});
			::h.asyncCall("show", [null, 1, { a = "b" }]);
			::h.call("hide", null);
			"#,
			"t",
		)
		.unwrap();
		assert_eq!(
			page.ran.borrow().as_slice(),
			[
				r#"["t","callFromSQ","MainMenuScreen","","onConnection",1]"#,
				r#"["t","callFromSQ","MainMenuScreen","MainMenuModule","onConnection",2]"#,
				r#"["t","callFromSQ","MainMenuScreen","","show",[null,1,{"a":"b"}]]"#,
				r#"["t","callFromSQ","MainMenuScreen","","hide",null]"#,
			]
		);
		assert_eq!(
			page.replies.borrow()[0],
			Reply::Void,
			"asyncCallSQ never resolves"
		);
		bridge.clear();
	}

	#[test]
	fn data_js_cannot_hold_arrives_as_undefined() {
		let (vm, bridge, page, _protocol) = setup();
		vm.run_script(
			r#"
			::h <- ::UI.connect("WorldScreen", { function onScreenConnected() {} });
			::h.asyncCall("setData", function () {});
			"#,
			"t",
		)
		.unwrap();
		assert_eq!(
			page.ran.borrow()[1],
			r#"["t","callFromSQ","WorldScreen","","setData"]"#
		);
		bridge.clear();
	}

	#[test]
	fn call_sq_replies_with_the_script_result() {
		let (vm, bridge, _page, protocol) = setup();
		vm.run_script(
			r#"
			::screen <- {
				function onScreenConnected() {}
				function onQuery(_data) { return { got = _data[1] * 2 }; }
				function onNothing() {}
			};
			::h <- ::UI.connect("TooltipScreen", ::screen);
			"#,
			"t",
		)
		.unwrap();
		let reply = protocol.handle_message(
			&vm,
			&bridge,
			&message(r#"["callSQ",3,1,"onQuery",[0,21.9]]"#),
		);
		assert_eq!(
			reply,
			Reply::Value(message(r#"{"got":42}"#)),
			"21.9 arrives as 21"
		);
		let reply =
			protocol.handle_message(&vm, &bridge, &message(r#"["callSQ",4,1,"onNothing"]"#));
		assert_eq!(reply, Reply::Value(Json::Null));
		let reply = protocol.handle_message(&vm, &bridge, &message(r#"["callSQ",5,1,"missing"]"#));
		assert_eq!(reply, Reply::Value(Json::Null));
		bridge.clear();
	}

	#[test]
	fn a_raising_script_resolves_without_a_value() {
		let (vm, bridge, page, _) = setup();
		let log = Rc::new(RefCell::new(Vec::new()));
		let sink = log.clone();
		let protocol = UiProtocol::new(page, move |m| sink.borrow_mut().push(m.to_string()));
		protocol.attach(&bridge);
		vm.run_script(
			r#"::h <- ::UI.connect("DialogScreen", { function onScreenConnected() {} function boom() { throw "x"; } });"#,
			"t",
		)
		.unwrap();
		let reply = protocol.handle_message(&vm, &bridge, &message(r#"["callSQ",3,1,"boom"]"#));
		assert_eq!(reply, Reply::NoValue);
		assert!(log.borrow().is_empty(), "only the VM's error handler reports it");
		bridge.clear();
	}

	#[test]
	fn registration_connects_the_root_screen() {
		let (vm, bridge, page, protocol) = setup();
		vm.run_script(
			r#"::GameInfo <- { function getVersionNumber() { return "1.5.2.3"; } function getVersionName() { return ""; } };"#,
			"t",
		)
		.unwrap();
		assert_eq!(
			protocol.handle_message(
				&vm,
				&bridge,
				&message(r#"["registerScreen",1,"RootScreen"]"#)
			),
			Reply::Void
		);
		protocol.handle_message(&vm, &bridge, &message(r#"["registrationFinished",2]"#));
		assert!(protocol.is_registered());
		assert_eq!(protocol.screens(), ["RootScreen"]);
		assert_eq!(
			page.ran.borrow().as_slice(),
			[
				r#"["t","callFromSQ","RootScreen","","onConnection",null]"#,
				r#"["t","callFromSQ","RootScreen","VersionModule","setVersion","Build: 1.5.2.3  - Date: 2026-06-22 15:00:00"]"#,
			]
		);
		bridge.clear();
	}

	#[test]
	fn unknown_handlers_are_errors() {
		let (vm, bridge, page, _) = setup();
		let protocol = UiProtocol::new(page, |_| {});
		let reply = protocol.handle_message(&vm, &bridge, &message(r#"["nope",3]"#));
		assert_eq!(
			reply.to_json().to_text(),
			r#"{"done":true,"errors":[{"first":2,"second":"no handler 'nope'"}]}"#
		);
		bridge.clear();
	}

	#[test]
	fn open_url_goes_to_the_opener_when_it_may() {
		let (vm, bridge, page, _) = setup();
		let log = Rc::new(RefCell::new(Vec::new()));
		let sink = log.clone();
		let protocol = UiProtocol::new(page, move |m| sink.borrow_mut().push(m.to_string()));
		let call = |url: &str| {
			let reply = protocol.handle_message(
				&vm,
				&bridge,
				&Json::Array(vec![Json::Str("openURL".into()), Json::Int(3), Json::Str(url.into())]),
			);
			assert_eq!(reply, Reply::Void, "the original never resolves openURL");
		};
		let nexus = "https://www.nexusmods.com/battlebrothers/mods/765";
		// No opener (tests, examples): only logged, as before.
		call(nexus);
		let opened = Rc::new(RefCell::new(Vec::new()));
		let sink = opened.clone();
		protocol.set_url_opener(move |url| {
			sink.borrow_mut().push(url.to_string());
			if url.ends_with("/fail") { Err("no launcher".into()) } else { Ok(()) }
		});
		call(nexus);
		call("file:///etc/passwd");
		call("https://example.com/a b");
		call("https://example.com/fail");
		call("www.battlebrothersgame.com");
		protocol.handle_message(&vm, &bridge, &message(r#"["openURL",4,42]"#));
		protocol.handle_message(&vm, &bridge, &message(r#"["openURL",5]"#));
		assert_eq!(
			opened.borrow().as_slice(),
			[nexus, "https://example.com/fail", "http://www.battlebrothersgame.com"]
		);
		assert_eq!(
			log.borrow().as_slice(),
			[
				format!("ui: openURL \"{nexus}\" (not opened)"),
				format!("ui: openURL \"{nexus}\" (opened)"),
				r#"ui: openURL "file:///etc/passwd" (refused: not an http(s) URL)"#.to_string(),
				r#"ui: openURL "https://example.com/a b" (refused: a character a URL can't hold)"#.to_string(),
				r#"ui: openURL "https://example.com/fail" (not opened: no launcher)"#.to_string(),
				r#"ui: openURL "www.battlebrothersgame.com" (opened)"#.to_string(),
				"ui: openURL 42 (refused: not text)".to_string(),
				"ui: openURL  (refused: not text)".to_string(),
			]
		);
		bridge.clear();
	}

	#[test]
	fn engine_handlers_answer_their_own_names() {
		let (vm, bridge, page, _) = setup();
		let log = Rc::new(RefCell::new(Vec::new()));
		let sink = log.clone();
		let protocol = UiProtocol::new(page, move |m| sink.borrow_mut().push(m.to_string()));
		let heard = Rc::new(RefCell::new(Vec::new()));
		let sink = heard.clone();
		protocol.set_engine_handler("bb.test", move |args| {
			sink.borrow_mut().push(Json::Array(args.to_vec()).to_text());
			Reply::Value(Json::Int(7))
		});
		let reply = protocol.handle_message(&vm, &bridge, &message(r#"["bb.test",3,"ignore","1.2.3"]"#));
		assert_eq!(reply, Reply::Value(Json::Int(7)));
		assert_eq!(heard.borrow().as_slice(), [r#"["ignore","1.2.3"]"#]);
		// A name of the engine's without a handler is unknown, as any other.
		let reply = protocol.handle_message(&vm, &bridge, &message(r#"["bb.other",4]"#));
		assert_eq!(reply, Reply::Error("no handler 'bb.other'".into()));
		assert_eq!(log.borrow().as_slice(), ["ui: engine.call to unknown handler 'bb.other'"]);
		bridge.clear();
	}

	#[test]
	#[should_panic(expected = "the engine's own handlers start with bb.")]
	fn engine_handlers_need_the_engines_prefix() {
		let (_vm, bridge, page, _) = setup();
		let protocol = UiProtocol::new(page, |_| {});
		bridge.clear();
		protocol.set_engine_handler("openURL", |_| Reply::Void);
	}

	#[test]
	fn disconnecting_a_module_tells_its_screen() {
		let (vm, bridge, page, _protocol) = setup();
		vm.run_script(
			r#"
			::h <- ::UI.connect("CharacterScreen", { function onScreenConnected() {} });
			::m <- ::h.connectToModule("Perks", {});
			::UI.disconnect(::m);
			"#,
			"t",
		)
		.unwrap();
		let ran = page.ran.borrow();
		assert_eq!(
			&ran[2..],
			[
				r#"["t","callFromSQNoArgs","CharacterScreen","Perks","onDisconnection"]"#,
				r#"["t","callFromSQNoArgs","CharacterScreen","","onModuleOnDisconnectionCalled"]"#,
			]
		);
		drop(ran);
		bridge.clear();
	}
}
