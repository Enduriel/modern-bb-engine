//! Headless UI bridge: a recording stand-in for the Coherent JS frontend.
//!
//! The real frontend is a WebView host that reproduces the Coherent GT
//! JS API. Headless, there is no JS — but the SQ-side UI scripts (loading_screen,
//! tactical_screen, turn_sequence_bar, character_screen, …) still run their full
//! logic, calling `UI.connect`/`JSHandle.asyncCall`/`call`/`connectToModule`. This
//! module provides those natives as a **recording** stub:
//!
//!   * `UI.connect(screen, sqObj)` / `JSHandle.connectToModule(moduleId, sqObj)`
//!     store the SQ object in a registry keyed by (screen, module) and hand back a
//!     `JSHandle` — so the harness can later invoke the SQ-side receiver methods
//!     directly (simulating UI input, exactly as the JS dispatcher would).
//!   * `JSHandle.asyncCall(method, data)` / `call(method, data)` append the call to
//!     an ordered log — so tests can assert what the SQ side told the UI to do.
//!
//! Together they let World/Tactical *logic* be tested deterministically with no
//! browser.
//!
//! A real UI host attaches a [`UiFrontend`] instead ([`UiBridge::set_frontend`]): the
//! calls then go to it rather than to the log, and the browser calls back into the
//! connected objects through their handle ids ([`UiBridge::receiver_for_handle`]).
//!
//! Like [`TimeService`](super::TimeService), the bridge is host-owned and holds
//! `SqStrongRef`s (the connected tables + recorded `data`), so **[`clear`](UiBridge::clear)
//! it (or drop it) before the VM is closed.**

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use squirrel::{ClassBuilder, Native, Number, Retained, SQVM, SqClass, SqStrongRef, SqTable};

/// One recorded SQ→JS call: what the SQ side asked the UI to do.
#[derive(Clone)]
pub struct UiCall {
	/// The screen the originating handle was connected to (`UI.connect` name).
	pub screen: String,
	/// The module id if the handle came from `connectToModule`, else `None`.
	pub module: Option<String>,
	/// The JS method invoked (e.g. `"show"`, `"setData"`).
	pub method: String,
	/// The data argument, retained for inspection (may reference `null`).
	pub data: SqStrongRef,
	/// `true` for the synchronous `JSHandle.call`, `false` for `asyncCall`.
	pub sync: bool,
}

/// A real UI on the other side of the bridge (the browser host): it receives every
/// SQ→JS call as it happens, in place of the headless call log.
///
/// In the original every one of these runs the UI's JS before returning, and that JS
/// may call back into the scripts (probed); `bridge` is how a frontend finds
/// the connected objects for those nested calls.
pub trait UiFrontend {
	/// `UI.connect(screen, obj)` (`module` is `None`) or
	/// `JSHandle.connectToModule(module, obj)`: `obj` is now reachable as `handle`.
	fn connect(&self, vm: &SQVM, bridge: &UiBridge, handle: u64, screen: &str, module: Option<&str>);
	/// `UI.disconnect` of the connection `handle`.
	fn disconnect(&self, vm: &SQVM, bridge: &UiBridge, handle: u64, screen: &str, module: Option<&str>);
	/// `JSHandle.asyncCall(method, data)` or `JSHandle.call(method, data)`: the same
	/// native in the original (both return `null` to the script).
	fn call(&self, vm: &SQVM, bridge: &UiBridge, screen: &str, module: Option<&str>, method: &str, data: &SqStrongRef);
}

/// Shared mutable bridge state: the connection registry, the SQ→JS call log, and
/// the `JSHandle` class (used to mint child handles in `connectToModule`).
struct BridgeState {
	connections: HashMap<(String, Option<String>), SqStrongRef>,
	/// Handle id → its connection key (what the UI passes back to call a script).
	handles: HashMap<u64, (String, Option<String>)>,
	next_handle: u64,
	log: Vec<UiCall>,
	/// Connections made since the last [`UiBridge::take_new_connections`].
	pending_connects: Vec<(String, Option<String>)>,
	class: Option<SqClass<JsHandle>>,
	frontend: Option<Rc<dyn UiFrontend>>,
	/// The cursor's screen position, as last set (`UI.setCursorPosition`, or the host's
	/// mouse handling).
	cursor: (i64, i64),
	/// Whether the cursor is over the UI rather than the game view, as the UI host last
	/// hit-tested it. With no host the UI covers everything.
	over_ui: bool,
}

impl BridgeState {
	fn new() -> Self {
		BridgeState {
			connections: HashMap::new(),
			handles: HashMap::new(),
			next_handle: 1,
			log: Vec::new(),
			pending_connects: Vec::new(),
			class: None,
			frontend: None,
			cursor: (0, 0),
			over_ui: true,
		}
	}

	/// Register `obj` as connected for `(screen, module)`; returns its new handle id
	/// and the frontend to tell (if any).
	fn connect(
		&mut self,
		screen: &str,
		module: Option<&str>,
		obj: SqStrongRef,
	) -> (u64, Option<Rc<dyn UiFrontend>>) {
		let key = (screen.to_string(), module.map(str::to_string));
		let id = self.next_handle;
		self.next_handle += 1;
		self.connections.insert(key.clone(), obj);
		self.handles.insert(id, key.clone());
		if self.frontend.is_none() {
			self.pending_connects.push(key);
		}
		(id, self.frontend.clone())
	}

	/// Record (headless) or hand over (frontend attached) one SQ→JS call.
	fn call(
		&mut self,
		h: &JsHandle,
		method: String,
		data: SqStrongRef,
		sync: bool,
	) -> Option<Rc<dyn UiFrontend>> {
		if self.frontend.is_some() {
			return self.frontend.clone();
		}
		self.log.push(UiCall {
			screen: h.screen.clone(),
			module: h.module.clone(),
			method,
			data,
			sync,
		});
		None
	}
}

/// The payload behind a `JSHandle` instance: which (screen, module) it targets and
/// a shared handle to the bridge state it records into.
#[derive(Clone)]
struct JsHandle {
	id: u64,
	screen: String,
	module: Option<String>,
	state: Rc<RefCell<BridgeState>>,
}

/// The headless UI bridge. Construct, [`install`](Self::install), then drive it
/// (simulate inputs via the connected receivers) and inspect it (the call log).
pub struct UiBridge {
	state: Rc<RefCell<BridgeState>>,
}

impl Default for UiBridge {
	fn default() -> Self {
		Self::new()
	}
}

impl UiBridge {
	/// A fresh bridge with an empty registry + log.
	pub fn new() -> Self {
		UiBridge {
			state: Rc::new(RefCell::new(BridgeState::new())),
		}
	}

	/// Register the `UI` global table and the `JSHandle` native class into the root.
	/// `registrationFinished` is implicitly already done — `UI.connect` never blocks.
	pub fn install(&self, vm: &mut SQVM) {
		// The recording JSHandle class. asyncCall/call append to the log; the data
		// arg is captured as a retained ref (any type, incl. null). connectToModule
		// records the sub-object and mints a child handle carrying the module id.
		let class = ClassBuilder::<JsHandle>::new("JSHandle")
			.method(
				"asyncCall",
				|h: &mut JsHandle, vm: &SQVM, method: String, data: Retained| {
					let frontend =
						h.state
							.borrow_mut()
							.call(h, method.clone(), data.0.clone(), false);
					if let Some(frontend) = frontend {
						let bridge = UiBridge { state: h.state.clone() };
						frontend.call(vm, &bridge, &h.screen, h.module.as_deref(), &method, &data.0);
					}
				},
			)
			.method(
				"call",
				|h: &mut JsHandle, vm: &SQVM, method: String, data: Retained| {
					// The same native as `asyncCall` in the original; the log keeps which
					// one a script used.
					let frontend =
						h.state
							.borrow_mut()
							.call(h, method.clone(), data.0.clone(), true);
					if let Some(frontend) = frontend {
						let bridge = UiBridge { state: h.state.clone() };
						frontend.call(vm, &bridge, &h.screen, h.module.as_deref(), &method, &data.0);
					}
				},
			)
			.method(
				"connectToModule",
				|h: &mut JsHandle, vm: &SQVM, module_id: String, obj: Retained| -> SqStrongRef {
					let (id, frontend, class) = {
						let mut st = h.state.borrow_mut();
						let (id, frontend) = st.connect(&h.screen, Some(&module_id), obj.0);
						(
							id,
							frontend,
							st.class.clone().expect("JSHandle class set during install"),
						)
					};
					let child = JsHandle {
						id,
						screen: h.screen.clone(),
						module: Some(module_id.clone()),
						state: h.state.clone(),
					};
					let handle = class.instantiate(vm, child);
					if let Some(frontend) = frontend {
						let bridge = UiBridge { state: h.state.clone() };
						frontend.connect(vm, &bridge, id, &h.screen, Some(&module_id));
					}
					handle
				},
			)
			.build(vm);
		self.state.borrow_mut().class = Some(class.clone());

		// UI.connect(screen, sqObj) -> JSHandle: record + mint a screen handle.
		let connect = {
			let state = self.state.clone();
			let class = class.clone();
			vm.make_closure(
				move |vm: &SQVM, screen: String, obj: Retained| -> SqStrongRef {
					let (id, frontend) = state.borrow_mut().connect(&screen, None, obj.0);
					let handle = class.instantiate(
						vm,
						JsHandle {
							id,
							screen: screen.clone(),
							module: None,
							state: state.clone(),
						},
					);
					if let Some(frontend) = frontend {
						let bridge = UiBridge { state: state.clone() };
						frontend.connect(vm, &bridge, id, &screen, None);
					}
					handle
				},
			)
		};

		// UI.disconnect(handle) -> null: forget the connection; null so callers can
		// do `m.JSHandle = UI.disconnect(m.JSHandle)`.
		let disconnect = {
			let state = self.state.clone();
			vm.make_closure(move |vm: &SQVM, handle: Option<Native<JsHandle>>| {
				if let Some(h) = handle {
					let frontend = {
						let mut st = state.borrow_mut();
						st.connections
							.remove(&(h.0.screen.clone(), h.0.module.clone()));
						st.handles.remove(&h.0.id);
						st.frontend.clone()
					};
					if let Some(frontend) = frontend {
						let bridge = UiBridge { state: state.clone() };
						frontend.disconnect(vm, &bridge, h.0.id, &h.0.screen, h.0.module.as_deref());
					}
				}
			})
		};

		// The cursor (the original's `0x53a050`, `0x53a100`, `0x539fe0`): its position
		// as last set, and whether it is over the UI, which the UI host decides.
		// `setCursorPosition` stores the position and answers that; `isMouseOver` reads
		// it. Without a host the UI covers everything.
		let set_cursor = vm.make_closure(|_name: String| {});
		let set_cursor_hardware = vm.make_closure(|value: bool| value);
		let is_cursor_hardware = vm.make_closure(|| {
			crate::native::stubs::note("Cursor.isHardware");
			false
		});
		let set_cursor_position = {
			let state = self.state.clone();
			vm.make_closure(move |x: Number, y: Number| {
				let mut st = state.borrow_mut();
				st.cursor = (x.0 as i64, y.0 as i64);
				st.over_ui
			})
		};
		let get_cursor_position = {
			let state = self.state.clone();
			vm.make_closure(move |vm: &SQVM| -> SqStrongRef {
				let (x, y) = state.borrow().cursor;
				let mut t = SqTable::new(vm);
				t.set("X", x);
				t.set("Y", y);
				t.to_strong()
			})
		};
		let is_mouse_over = {
			let state = self.state.clone();
			vm.make_closure(move || state.borrow().over_ui)
		};
		let get_cursor_size = vm.make_closure(|vm: &SQVM| -> SqStrongRef {
			let mut t = SqTable::new(vm);
			t.set("Width", 0i64);
			t.set("Height", 0i64);
			t.to_strong()
		});
		let get_cursor_offsets = vm.make_closure(|vm: &SQVM| -> SqStrongRef {
			let mut t = SqTable::new(vm);
			t.set("X", 0i64);
			t.set("Y", 0i64);
			t.to_strong()
		});

		let mut ui = SqTable::new(vm);
		ui.set("connect", connect);
		ui.set("disconnect", disconnect);
		ui.set("setCursor", set_cursor);
		ui.set("setCursorHardware", set_cursor_hardware);
		ui.set("isCursorHardware", is_cursor_hardware);
		ui.set("setCursorPosition", set_cursor_position);
		ui.set("getCursorPosition", get_cursor_position);
		ui.set("isMouseOver", is_mouse_over);
		ui.set("getCursorSize", get_cursor_size);
		ui.set("getCursorOffsets", get_cursor_offsets);
		SqTable::root(vm).set("UI", ui);
	}

	/// The host's cursor: its screen position and whether it is over the UI (what
	/// `UI.getCursorPosition`, `UI.isMouseOver` and `UI.setCursorPosition` report).
	pub fn set_cursor(&self, x: i64, y: i64, over_ui: bool) {
		let mut st = self.state.borrow_mut();
		st.cursor = (x, y);
		st.over_ui = over_ui;
	}

	/// The cursor's screen position, as last set.
	pub fn cursor(&self) -> (i64, i64) {
		self.state.borrow().cursor
	}

	/// Send every SQ→JS call to `frontend` from now on (instead of the call log).
	pub fn set_frontend(&self, frontend: Rc<dyn UiFrontend>) {
		self.state.borrow_mut().frontend = Some(frontend);
	}

	/// The connected object behind handle id `handle` (the id the frontend got in
	/// [`UiFrontend::connect`]), while it is connected.
	pub fn receiver_for_handle(&self, handle: u64) -> Option<SqStrongRef> {
		let st = self.state.borrow();
		let key = st.handles.get(&handle)?;
		st.connections.get(key).cloned()
	}

	// ---- harness / inspection API ----

	/// The SQ table connected for `screen` (with `module = None`) or for a module
	/// (`Some(id)`), if present — clone of the stored ref so a test can invoke its
	/// receiver methods (`ref.call_method(vm, "onOkPressed", ())`).
	pub fn receiver(&self, screen: &str, module: Option<&str>) -> Option<SqStrongRef> {
		self.state
			.borrow()
			.connections
			.get(&(screen.to_string(), module.map(str::to_string)))
			.cloned()
	}

	/// All recorded SQ→JS calls, in order (a cloned snapshot).
	pub fn calls(&self) -> Vec<UiCall> {
		self.state.borrow().log.clone()
	}

	/// The `data` payloads of every recorded call to `screen.method` (in order).
	pub fn calls_to(&self, screen: &str, method: &str) -> Vec<SqStrongRef> {
		self.state
			.borrow()
			.log
			.iter()
			.filter(|c| c.screen == screen && c.method == method)
			.map(|c| c.data.clone())
			.collect()
	}

	/// The ordered JS method names recorded for `screen` (to assert call ordering).
	pub fn methods_for(&self, screen: &str) -> Vec<String> {
		self.state
			.borrow()
			.log
			.iter()
			.filter(|c| c.screen == screen)
			.map(|c| c.method.clone())
			.collect()
	}

	/// Take the recorded SQ→JS calls, leaving the log empty (for a client that consumes
	/// them every frame; see [`crate::ui_client`]).
	pub fn take_calls(&self) -> Vec<UiCall> {
		std::mem::take(&mut self.state.borrow_mut().log)
	}

	/// Take the `(screen, module)` connections made since the last call.
	pub fn take_new_connections(&self) -> Vec<(String, Option<String>)> {
		std::mem::take(&mut self.state.borrow_mut().pending_connects)
	}

	/// Number of recorded SQ→JS calls.
	pub fn call_count(&self) -> usize {
		self.state.borrow().log.len()
	}

	/// Number of open connections (`UI.connect` not yet disconnected).
	pub fn connection_count(&self) -> usize {
		self.state.borrow().handles.len()
	}

	/// Release every retained ref (connections, recorded data, the class). Call this
	/// before the VM is closed (the drop-before-close contract for SqStrongRefs).
	pub fn clear(&self) {
		let mut st = self.state.borrow_mut();
		st.connections.clear();
		st.handles.clear();
		st.log.clear();
		st.pending_connects.clear();
		st.class = None;
		st.frontend = None;
	}
}
