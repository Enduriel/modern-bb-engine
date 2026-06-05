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

/// Shared mutable bridge state: the connection registry, the SQ→JS call log, and
/// the `JSHandle` class (used to mint child handles in `connectToModule`).
struct BridgeState {
	connections: HashMap<(String, Option<String>), SqStrongRef>,
	log: Vec<UiCall>,
	class: Option<SqClass<JsHandle>>,
}

impl BridgeState {
	fn new() -> Self {
		BridgeState {
			connections: HashMap::new(),
			log: Vec::new(),
			class: None,
		}
	}
}

/// The payload behind a `JSHandle` instance: which (screen, module) it targets and
/// a shared handle to the bridge state it records into.
#[derive(Clone)]
struct JsHandle {
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
				|h: &mut JsHandle, method: String, data: Retained| {
					let call = UiCall {
						screen: h.screen.clone(),
						module: h.module.clone(),
						method,
						data: data.0,
						sync: false,
					};
					h.state.borrow_mut().log.push(call);
				},
			)
			.method(
				"call",
				|h: &mut JsHandle, method: String, data: Retained| {
					// Return value is unused by all (turn_sequence_bar) call sites → null.
					let call = UiCall {
						screen: h.screen.clone(),
						module: h.module.clone(),
						method,
						data: data.0,
						sync: true,
					};
					h.state.borrow_mut().log.push(call);
				},
			)
			.method(
				"connectToModule",
				|h: &mut JsHandle, vm: &SQVM, module_id: String, obj: Retained| -> SqStrongRef {
					let child = JsHandle {
						screen: h.screen.clone(),
						module: Some(module_id.clone()),
						state: h.state.clone(),
					};
					let class = {
						let mut st = h.state.borrow_mut();
						st.connections
							.insert((h.screen.clone(), Some(module_id)), obj.0);
						st.class.clone().expect("JSHandle class set during install")
					};
					class.instantiate(vm, child)
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
					state
						.borrow_mut()
						.connections
						.insert((screen.clone(), None), obj.0);
					class.instantiate(
						vm,
						JsHandle {
							screen,
							module: None,
							state: state.clone(),
						},
					)
				},
			)
		};

		// UI.disconnect(handle) -> null: forget the connection; null so callers can
		// do `m.JSHandle = UI.disconnect(m.JSHandle)`.
		let disconnect = {
			let state = self.state.clone();
			vm.make_closure(move |handle: Option<Native<JsHandle>>| {
				if let Some(h) = handle {
					state
						.borrow_mut()
						.connections
						.remove(&(h.0.screen.clone(), h.0.module.clone()));
				}
			})
		};

		// Cursor natives — no-op / false stubs (no windowing headless).
		let set_cursor = vm.make_closure(|_name: String| {});
		let set_cursor_hardware = vm.make_closure(|value: bool| value);
		let is_cursor_hardware = vm.make_closure(|| false);
		let set_cursor_position = vm.make_closure(|_x: Number, _y: Number| false);
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
		ui.set("getCursorSize", get_cursor_size);
		ui.set("getCursorOffsets", get_cursor_offsets);
		SqTable::root(vm).set("UI", ui);
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

	/// Number of recorded SQ→JS calls.
	pub fn call_count(&self) -> usize {
		self.state.borrow().log.len()
	}

	/// Release every retained ref (connections, recorded data, the class). Call this
	/// before the VM is closed (the drop-before-close contract for SqStrongRefs).
	pub fn clear(&self) {
		let mut st = self.state.borrow_mut();
		st.connections.clear();
		st.log.clear();
		st.class = None;
	}
}
