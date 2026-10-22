//! The [`RootState`] named-state stack — the game's top-level orchestrator.
//!
//! `scripts/states/state.nut` is only virtual lifecycle hooks (onInit/onShow/…);
//! the *machinery* — `add`/`get`/`getName`/`show`/`hide`/`finish`/
//! `sendMessageToSiblings`, plus the `Root` background-task hook — are **native
//! methods the engine injects onto state instances**, and `RootState`/`Root` are
//! globals it sets. This module is that engine piece.
//!
//! Injection pattern (the reusable one for the larger entity-method subsystems):
//! the native methods are built **once** at [`install`](RootState::install) (which
//! needs `&mut SQVM` for closure creation) and stored as shared closures; at
//! runtime they are dropped onto each state table as ordinary slots (`set`, which
//! needs only `&SQVM`). A shared method recovers *which* state it was invoked on
//! via [`SQVM::this_identity`] (the receiver's pointer-identity), looked up in the
//! Rust-side registry. All `add` sites target the root's child list (verified in
//! vanilla), so `add`/`get` ignore their receiver.
//!
//! Host-owned like [`TimeService`](crate::TimeService): it holds [`SqStrongRef`]s
//! (the states + the shared method closures), so **[`clear`](RootState::clear) it
//! (or drop it) before the VM is closed.**

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef, SqTable};

use crate::ScriptErrors;
use crate::input_events::{InputClasses, KeyInput, MouseInput};
use crate::module::Instantiator;

/// A registered child state of the root (a sibling among the root's children).
struct Child {
	name: String,
	id: i64,
	state: SqStrongRef,
	frame: FrameHook,
	/// `finish()` was called: the next update pass finishes it (see [`RootState::update`]).
	finishing: bool,
}

/// The root state itself (the container; not one of its own children). It runs no
/// frames: in the original its `onUpdate` and `onRender` are never called (probed
/// 2026-10-19: hooked on root_state, neither ran once through the boot, the menu, a
/// campaign's start and the way back to the menu).
struct Root {
	id: i64,
	state: SqStrongRef,
	/// Hidden through its `onInit`, shown from then on (see [`RootState::boot`]).
	hidden: bool,
}

/// A state's per-frame hooks. Like the original, `onUpdate` and `onRender` are captured
/// when the state is registered (replacing them on the instance later has no effect),
/// and only a state that isn't hidden runs them (both probed 2026-10-13).
///
/// A child starts hidden: it updates, renders and gets input only once it `show()`s
/// itself (vanilla's do at the end of their `init()`). In the original, `isVisible()`
/// answers false from `RootState.add` through the state's `onInit`, and none of a
/// steady stream of pointer moves reaches it then (probed 2026-10-19). The root too
/// is hidden through its `onInit`; the engine shows it (`onShow`) right after
/// (probed 2026-10-19, see [`RootState::boot`]).
struct FrameHook {
	on_update: Option<SqStrongRef>,
	on_render: Option<SqStrongRef>,
	hidden: bool,
}

impl FrameHook {
	/// Capture `state.onUpdate` and `state.onRender`, bound to the state, which starts
	/// `hidden` or not.
	fn capture(vm: &SQVM, state: &SqStrongRef, hidden: bool) -> Self {
		let bound = |name: &str| {
			let table = state.as_table(vm)?;
			state.get_member::<SqStrongRef>(vm, name)?.bindenv(vm, &table)
		};
		FrameHook {
			on_update: bound("onUpdate"),
			on_render: bound("onRender"),
			hidden,
		}
	}
}

/// Shared state-stack machine: the registry, the constructor, the background-task
/// hook, and the shared native method closures (to inject onto each new state).
struct Machine {
	children: Vec<Child>,
	root: Option<Root>,
	instantiate: Instantiator,
	bg_callback: Option<SqStrongRef>,
	bg_suspended: bool,
	/// What `State.updateScene()` runs: the host's scene updates (world thinkers, …).
	scene_update: Option<SceneUpdate>,
	/// What `State.renderScene()` runs: the logic side of rendering (discovery, …).
	scene_render: Option<SceneUpdate>,
	/// The engine's own listener, which sees every mouse event before the states (the
	/// world's hovered tile).
	mouse_listener: Option<MouseListener>,
	/// Added states whose `onInit` hasn't run yet.
	pending_init: Vec<SqStrongRef>,
	methods: Vec<(&'static str, SqStrongRef)>,
	/// The mouse and key event classes (`onMouseInput` / `onKeyInput`).
	input: Option<InputClasses>,
	/// What an error raised in a state's hook does (see [`ScriptErrors`]).
	script_errors: ScriptErrors,
}

/// A host scene update, run by `State.updateScene()`.
type SceneUpdate = Rc<dyn Fn(&SQVM) -> Result<(), String>>;
type MouseListener = Rc<dyn Fn(&SQVM, &MouseInput) -> Result<(), String>>;

impl Machine {
	/// The registered name of the state with identity `id` (`"RootState"` for the
	/// root, the child's add-name for a child, `""` if unknown).
	fn name_of(&self, id: i64) -> String {
		if self.root.as_ref().is_some_and(|r| r.id == id) {
			return "RootState".to_string();
		}
		self.children
			.iter()
			.find(|c| c.id == id)
			.map(|c| c.name.clone())
			.unwrap_or_default()
	}

	/// The state with identity `id` (root or a child).
	fn by_id(&self, id: i64) -> Option<SqStrongRef> {
		if let Some(root) = &self.root {
			if root.id == id {
				return Some(root.state.clone());
			}
		}
		self.children
			.iter()
			.find(|c| c.id == id)
			.map(|c| c.state.clone())
	}

	/// The child state registered under `name`.
	fn get(&self, name: &str) -> Option<SqStrongRef> {
		self.children
			.iter()
			.find(|c| c.name == name)
			.map(|c| c.state.clone())
	}

	/// The other children (everyone but identity `id`) — the sibling set.
	fn siblings_of(&self, id: i64) -> Vec<SqStrongRef> {
		self.children
			.iter()
			.filter(|c| c.id != id)
			.map(|c| c.state.clone())
			.collect()
	}

	/// Whether the state with identity `id` is visible (not hidden).
	fn is_visible(&self, id: i64) -> bool {
		if let Some(root) = self.root.as_ref().filter(|r| r.id == id) {
			return !root.hidden;
		}
		self.children.iter().find(|c| c.id == id).is_some_and(|c| !c.frame.hidden)
	}

	/// Mark the state with identity `id` hidden or shown.
	fn set_hidden(&mut self, id: i64, hidden: bool) {
		if let Some(root) = self.root.as_mut().filter(|r| r.id == id) {
			root.hidden = hidden;
		} else if let Some(child) = self.children.iter_mut().find(|c| c.id == id) {
			child.frame.hidden = hidden;
		}
	}

	/// A captured hook of every visible child, in registration order (the root runs no
	/// frames, see [`Root`]).
	fn visible_hooks(&self, hook: fn(&FrameHook) -> Option<SqStrongRef>) -> Vec<SqStrongRef> {
		self.children
			.iter()
			.map(|c| &c.frame)
			.filter(|f| !f.hidden)
			.filter_map(hook)
			.collect()
	}

	/// Remove the child with identity `id`, returning it (root is never removed).
	fn remove(&mut self, id: i64) -> Option<SqStrongRef> {
		let pos = self.children.iter().position(|c| c.id == id)?;
		Some(self.children.remove(pos).state)
	}
}

/// Ignore a "no method" error (the script didn't define the optional hook), but
/// propagate a real error raised inside one that does.
fn ignore_missing(result: Result<(), String>) -> Result<(), String> {
	match result {
		Err(e) if e.starts_with("no method") => Ok(()),
		other => other,
	}
}

/// Construct `path`, inject the state methods, register it as a child, tell the other
/// children (`onSiblingAdded(name)`), and queue its `onInit` for the next update. As in
/// the original, the caller gets the new state back before it initializes: e.g. the
/// world state hands the new tactical state its battle (`setStrategicProperties`), and
/// the main menu hands the world state its campaign settings from `onSiblingAdded`,
/// before their `onInit`s read them. No machine borrow is held across the
/// script-running calls (they re-enter).
fn add_child(
	machine: &Rc<RefCell<Machine>>,
	vm: &SQVM,
	name: &str,
	path: &str,
) -> Result<SqStrongRef, String> {
	let instantiate = machine.borrow().instantiate.clone();
	let child = instantiate(vm, path)?;
	let id = child.identity();
	inject(machine, vm, &child);
	// Hidden until it shows itself (see `FrameHook`).
	let frame = FrameHook::capture(vm, &child, true);
	machine.borrow_mut().children.push(Child {
		name: name.to_string(),
		id,
		state: child.clone(),
		frame,
		finishing: false,
	});
	let (siblings, errors) = {
		let m = machine.borrow();
		(m.siblings_of(id), m.script_errors)
	};
	for sibling in siblings {
		errors.settle(ignore_missing(sibling.call_method(vm, "onSiblingAdded", (name.to_string(),))))?;
	}
	machine.borrow_mut().pending_init.push(child.clone());
	Ok(child)
}

/// Run the queued `onInit`s, including those of states added meanwhile.
fn init_pending(machine: &Rc<RefCell<Machine>>, vm: &SQVM) -> Result<(), String> {
	loop {
		let (next, errors) = {
			let mut m = machine.borrow_mut();
			if m.pending_init.is_empty() {
				return Ok(());
			}
			(m.pending_init.remove(0), m.script_errors)
		};
		errors.settle(next.call_method(vm, "onInit", ()))?;
	}
}

/// Drop the shared native methods onto `state` as ordinary slots (`set` needs only
/// `&SQVM`; the closures were built once at install).
fn inject(machine: &Rc<RefCell<Machine>>, vm: &SQVM, state: &SqStrongRef) {
	let methods: Vec<(&'static str, SqStrongRef)> = machine
		.borrow()
		.methods
		.iter()
		.map(|(n, c)| (*n, c.clone()))
		.collect();
	if let Some(mut table) = state.as_table(vm) {
		for (name, closure) in methods {
			table.set(name, closure);
		}
	}
}

/// The native state-stack subsystem.
pub struct RootState {
	machine: Rc<RefCell<Machine>>,
}

impl RootState {
	/// Create the subsystem. `instantiate` is the `this.new` capability (get it from
	/// [`ModuleSystem::instantiator`](crate::ModuleSystem::instantiator)).
	pub fn new(instantiate: Instantiator) -> Self {
		RootState {
			machine: Rc::new(RefCell::new(Machine {
				children: Vec::new(),
				root: None,
				instantiate,
				bg_callback: None,
				bg_suspended: false,
				scene_update: None,
				scene_render: None,
				mouse_listener: None,
				pending_init: Vec::new(),
				methods: Vec::new(),
				input: None,
				script_errors: ScriptErrors::Fail,
			})),
		}
	}

	/// Build the shared native state methods. Call once, with `&mut SQVM`, before
	/// [`boot`](Self::boot).
	pub fn install(&self, vm: &mut SQVM) {
		self.machine.borrow_mut().input = Some(InputClasses::build(vm));
		// Each closure captures the shared machine; a method recovers its receiver
		// via `vm.this_identity()`. `add`/`get` ignore the receiver (all `add` sites
		// target the root's children).
		let get_name = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> String { m.borrow().name_of(vm.this_identity()) })
		};
		let add = {
			let m = self.machine.clone();
			vm.make_closure(
				move |vm: &SQVM, name: String, path: String| -> Result<SqStrongRef, String> {
					add_child(&m, vm, &name, &path)
				},
			)
		};
		let get = {
			let m = self.machine.clone();
			vm.make_closure(move |name: String| -> Option<SqStrongRef> { m.borrow().get(&name) })
		};
		let show = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let id = vm.this_identity();
				m.borrow_mut().set_hidden(id, false);
				let (target, errors) = (m.borrow().by_id(id), m.borrow().script_errors);
				errors.settle(target.map_or(Ok(()), |t| t.call_method(vm, "onShow", ())))
			})
		};
		let hide = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let id = vm.this_identity();
				m.borrow_mut().set_hidden(id, true);
				let (target, errors) = (m.borrow().by_id(id), m.borrow().script_errors);
				errors.settle(target.map_or(Ok(()), |t| t.call_method(vm, "onHide", ())))
			})
		};
		// finish(): a child only gets flagged; the update pass finishes it in its turn
		// (disassembled). Until then it's still a sibling and still found by `get`.
		let finish = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let id = vm.this_identity();
				if let Some(child) = m.borrow_mut().children.iter_mut().find(|c| c.id == id) {
					child.finishing = true;
					return Ok(());
				}
				let target = m.borrow_mut().remove(id);
				let errors = m.borrow().script_errors;
				errors.settle(target.map_or(Ok(()), |t| t.call_method(vm, "onFinish", ())))
			})
		};
		let send_to_siblings = {
			let m = self.machine.clone();
			vm.make_closure(
				move |vm: &SQVM, msg: squirrel::Retained| -> Result<(), String> {
					// Each sibling hears `onSiblingSentMessage(senderName, message)`.
					let (sender, siblings, errors) = {
						let machine = m.borrow();
						let id = vm.this_identity();
						(machine.name_of(id), machine.siblings_of(id), machine.script_errors)
					};
					for sibling in siblings {
						errors.settle(ignore_missing(sibling.call_method(
							vm,
							"onSiblingSentMessage",
							(sender.clone(), msg.0.clone()),
						)))?;
					}
					Ok(())
				},
			)
		};
		let set_bg_callback = {
			let m = self.machine.clone();
			vm.make_closure(move |callback: Option<SqStrongRef>| {
				m.borrow_mut().bg_callback = callback;
			})
		};
		let is_bg_suspended = {
			let m = self.machine.clone();
			vm.make_closure(move || -> bool { m.borrow().bg_suspended })
		};

		let is_visible = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| m.borrow().is_visible(vm.this_identity()))
		};
		// updateScene(): the original updates every scene (sprite animation, and the world
		// scene's entity thinkers). Headless, only the host's logic updates run.
		let update_scene = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let hook = m.borrow().scene_update.clone();
				hook.map_or(Ok(()), |f| f(vm))
			})
		};
		// The parent of a child is the root; the root has none.
		let get_parent = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Option<SqStrongRef> {
				let machine = m.borrow();
				let root = machine.root.as_ref()?;
				(root.id != vm.this_identity()).then(|| root.state.clone())
			})
		};
		let has_children = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| {
				let machine = m.borrow();
				machine.root.as_ref().is_some_and(|r| r.id == vm.this_identity()) && !machine.children.is_empty()
			})
		};
		// finishChildren(): finish every child of the root (children have none).
		let finish_children = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let is_root = m.borrow().root.as_ref().is_some_and(|r| r.id == vm.this_identity());
				if !is_root {
					return Ok(());
				}
				let ids: Vec<i64> = m.borrow().children.iter().map(|c| c.id).collect();
				for id in ids {
					let target = m.borrow_mut().remove(id);
					if let Some(t) = target {
						let errors = m.borrow().script_errors;
						errors.settle(t.call_method(vm, "onFinish", ()))?;
					}
				}
				Ok(())
			})
		};
		// renderScene(): the original draws every scene; what drawing decides for the
		// game (which entities the player discovers) is the host's render hook.
		let render_scene = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let hook = m.borrow().scene_render.clone();
				hook.map_or(Ok(()), |f| f(vm))
			})
		};
		// Draw order doesn't exist headless.
		let to_front = crate::native::stubs::closure(vm, "RootState.state.toFront");
		let to_back = crate::native::stubs::closure(vm, "RootState.state.toBack");

		self.machine.borrow_mut().methods = vec![
			("isVisible", is_visible),
			("updateScene", update_scene),
			("renderScene", render_scene),
			("toFront", to_front),
			("toBack", to_back),
			("getParent", get_parent),
			("hasChildren", has_children),
			("finishChildren", finish_children),
			("getName", get_name),
			("add", add),
			("get", get),
			("show", show),
			("hide", hide),
			("finish", finish),
			("sendMessageToSiblings", send_to_siblings),
			("setBackgroundTaskCallback", set_bg_callback),
			("isBackgroundTaskSuspended", is_bg_suspended),
		];
	}

	/// Create the root state at `root_path` (normally `"scripts/root_state"`), inject
	/// the state methods, publish it as the `RootState` global, and run its `onInit`
	/// — which builds the global UI objects and `add`s the first child state — then
	/// show it (its `onShow`), as the original does: in the original's log
	/// `RootState::onShow()` (state.nut's own `onShow`) follows the root's `onInit` at
	/// once, and a hooked root sees `isVisible()` false in `onInit`, true in `onShow`,
	/// both before the first update (probed 2026-10-19). Returns the root. Call
	/// [`install`](Self::install) first.
	pub fn boot(&self, vm: &SQVM, root_path: &str) -> Result<SqStrongRef, String> {
		let instantiate = self.machine.borrow().instantiate.clone();
		let root = instantiate(vm, root_path)?;
		let id = root.identity();
		inject(&self.machine, vm, &root);
		self.machine.borrow_mut().root = Some(Root {
			id,
			state: root.clone(),
			hidden: true,
		});
		SqTable::root(vm).set("RootState", root.clone());
		root.call_method(vm, "onInit", ())?;
		// Then the root is shown, at once: its `onShow` runs right after its `onInit`
		// returns, before the first update (and so before the `onInit` of the states it
		// added). Hidden during its `onInit` (`isVisible()` false), shown from then on.
		self.machine.borrow_mut().set_hidden(id, false);
		root.call_method(vm, "onShow", ())?;
		// MapGen exists now: the world generator follows the size settings.
		crate::native::world_size::follow_settings(vm)?;
		// The engine's first update would run the states the root added.
		init_pending(&self.machine, vm)?;
		Ok(root)
	}

	// ---- host / inspection API ----

	/// Run the `onInit` of every state added since the last update (the original runs
	/// them on its next update; a host driving the boot itself calls this instead).
	pub fn init_pending(&self, vm: &SQVM) -> Result<(), String> {
		init_pending(&self.machine, vm)
	}

	/// Mark the child state `name` shown without running its `onShow`: for a host that
	/// drives a state's start itself in place of the script's own `init()`, which would
	/// end with `show()`. False if there is no such state.
	pub fn mark_shown(&self, name: &str) -> bool {
		let mut machine = self.machine.borrow_mut();
		let Some(child) = machine.children.iter_mut().find(|c| c.name == name) else {
			return false;
		};
		child.frame.hidden = false;
		true
	}

	/// What an error raised in a state's hook does from now on: fail the call that ran
	/// it (the frame, the native), or go on as the original's (see [`ScriptErrors`]).
	pub fn set_script_errors(&self, policy: ScriptErrors) {
		self.machine.borrow_mut().script_errors = policy;
	}

	/// Set what `State.updateScene()` runs each frame (the host's scene updates).
	pub fn set_scene_update(&self, update: impl Fn(&SQVM) -> Result<(), String> + 'static) {
		self.machine.borrow_mut().scene_update = Some(Rc::new(update));
	}

	/// Set what `State.renderScene()` runs each frame (the logic side of rendering).
	pub fn set_scene_render(&self, render: impl Fn(&SQVM) -> Result<(), String> + 'static) {
		self.machine.borrow_mut().scene_render = Some(Rc::new(render));
	}

	/// Set the engine's mouse listener, run on every mouse event before the states'
	/// `onMouseInput`.
	pub fn set_mouse_listener(&self, listener: impl Fn(&SQVM, &MouseInput) -> Result<(), String> + 'static) {
		self.machine.borrow_mut().mouse_listener = Some(Rc::new(listener));
	}

	/// One frame's update pass, as the original's: each child in registration order (the
	/// root runs no `onUpdate`, see [`Root`]). A child runs its `onUpdate` (if visible)
	/// unless it is finishing; then, if it is finishing (even by its own update), its
	/// `onFinish` runs and it leaves the list. States added during the pass wait for the
	/// next one.
	pub fn update(&self, vm: &SQVM) -> Result<(), String> {
		init_pending(&self.machine, vm)?;
		let ids: Vec<i64> = self.machine.borrow().children.iter().map(|c| c.id).collect();
		for id in ids {
			let hook = {
				let m = self.machine.borrow();
				let Some(c) = m.children.iter().find(|c| c.id == id) else { continue };
				(!c.finishing && !c.frame.hidden).then(|| c.frame.on_update.clone()).flatten()
			};
			if let Some(on_update) = hook {
				self.settle(on_update.call(vm, ()))?;
			}
			let finishing = self.machine.borrow().children.iter().any(|c| c.id == id && c.finishing);
			if finishing {
				let state = self.machine.borrow_mut().remove(id);
				if let Some(state) = state {
					self.settle(state.call_method(vm, "onFinish", ()))?;
				}
			}
		}
		Ok(())
	}

	/// Run `onRender` on every visible child, as the original does after the updates.
	pub fn render(&self, vm: &SQVM) -> Result<(), String> {
		let renders = self.machine.borrow().visible_hooks(|f| f.on_render.clone());
		for on_render in renders {
			self.settle(on_render.call(vm, ()))?;
		}
		Ok(())
	}

	/// Hand a mouse event to the visible states' `onMouseInput`, the most recently added
	/// first and the root last, until one answers true (handled). Whether the original
	/// routes it this way among several visible states is unverified; in a battle or on
	/// the world map only one state with input handling is visible. A state added but
	/// not shown yet gets none (see `FrameHook`): mods that hook `onMouseInput` rely on
	/// its `init()` having run (Configurable Pause reads the player's party).
	pub fn mouse_input(&self, vm: &SQVM, input: MouseInput) -> Result<bool, String> {
		let listener = self.machine.borrow().mouse_listener.clone();
		if let Some(listener) = listener {
			listener(vm, &input)?;
		}
		let event = {
			let m = self.machine.borrow();
			m.input.as_ref().ok_or("mouse_input: not installed")?.mouse(vm, input)
		};
		self.dispatch(vm, "onMouseInput", &event)
	}

	/// Hand a key event to the visible states' `onKeyInput` (see [`mouse_input`](Self::mouse_input)).
	pub fn key_input(&self, vm: &SQVM, input: KeyInput) -> Result<bool, String> {
		let event = {
			let m = self.machine.borrow();
			m.input.as_ref().ok_or("key_input: not installed")?.key(vm, input)
		};
		self.dispatch(vm, "onKeyInput", &event)
	}

	fn dispatch(&self, vm: &SQVM, method: &str, event: &SqStrongRef) -> Result<bool, String> {
		let targets: Vec<SqStrongRef> = {
			let m = self.machine.borrow();
			let children = m.children.iter().rev().filter(|c| !c.frame.hidden).map(|c| c.state.clone());
			children.chain(m.root.iter().filter(|r| !r.hidden).map(|r| r.state.clone())).collect()
		};
		for state in targets {
			// A hook that threw handled nothing: the next state gets the event.
			if self.settle(state.call_method_ret::<bool, _>(vm, method, (event.clone(),)))?.unwrap_or(false) {
				return Ok(true);
			}
		}
		Ok(false)
	}

	/// Run the registered background task (`Root.setBackgroundTaskCallback`) once, as
	/// the original does after the states' updates every frame, whether or not the
	/// state that registered it is visible.
	///
	/// The original queues it for a worker thread after the states' updates
	/// (`0x521a2b`, clearing the flag `isBackgroundTaskSuspended` answers, `+0x11`), so
	/// it thinks while the frame draws, and suspends it when the next frame starts
	/// (`0x521910`: sets the flag, then waits for the task to return). The scripts return
	/// once suspended and past their allotment (`isAllottedTimeReached`:
	/// `Const.AI.AllottedTimePerEvaluationRun`, 5 ms of `Time.getExactTime`), and go on
	/// where they were the next time. Run inline, the frame needs the VM back at once, so
	/// the task runs suspended from the start. Under the live exact clock an AI
	/// evaluation yields after its 5 ms and spreads over frames, as the original's (which
	/// gets the rest of its frame on top). Under the frame clock no time passes within a
	/// call, so nothing yields for time (tests, traces).
	pub fn run_background(&self, vm: &SQVM) -> Result<(), String> {
		let callback = self.machine.borrow().bg_callback.clone();
		let Some(callback) = callback else { return Ok(()) };
		let was = std::mem::replace(&mut self.machine.borrow_mut().bg_suspended, true);
		let result = callback.call(vm, ());
		self.machine.borrow_mut().bg_suspended = was;
		self.settle(result)
	}

	/// `result` of a state's hook, as the policy leaves it (see [`ScriptErrors`]).
	fn settle<T: Default>(&self, result: Result<T, String>) -> Result<T, String> {
		let errors = self.machine.borrow().script_errors;
		errors.settle(result)
	}

	/// The root state instance, if booted.
	pub fn root(&self) -> Option<SqStrongRef> {
		self.machine.borrow().root.as_ref().map(|r| r.state.clone())
	}

	/// The child state registered under `name`.
	pub fn state(&self, name: &str) -> Option<SqStrongRef> {
		self.machine.borrow().get(name)
	}

	/// The names of the root's child states, in add order.
	pub fn state_names(&self) -> Vec<String> {
		self.machine
			.borrow()
			.children
			.iter()
			.map(|c| c.name.clone())
			.collect()
	}

	/// The registered background-task callback (the cooperative-yield hook), if set —
	/// the host fires this during long generation tasks (worldgen) between
	/// `isBackgroundTaskSuspended` checkpoints.
	pub fn background_callback(&self) -> Option<SqStrongRef> {
		self.machine.borrow().bg_callback.clone()
	}

	/// Set the `isBackgroundTaskSuspended` flag the scripts poll.
	pub fn set_suspended(&self, suspended: bool) {
		self.machine.borrow_mut().bg_suspended = suspended;
	}

	/// Release every retained ref (states + method closures + callback). Call before
	/// the VM is closed.
	pub fn clear(&self) {
		let mut machine = self.machine.borrow_mut();
		machine.children.clear();
		machine.root = None;
		machine.pending_init.clear();
		machine.input = None;
		machine.scene_update = None;
		machine.scene_render = None;
		machine.mouse_listener = None;
		machine.bg_callback = None;
		machine.methods.clear();
	}
}
