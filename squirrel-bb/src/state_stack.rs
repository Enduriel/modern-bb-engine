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

use crate::module::Instantiator;

/// A registered child state of the root (a sibling among the root's children).
struct Child {
	name: String,
	id: i64,
	state: SqStrongRef,
}

/// The root state itself (the container; not one of its own children).
struct Root {
	id: i64,
	state: SqStrongRef,
}

/// Shared state-stack machine: the registry, the constructor, the background-task
/// hook, and the shared native method closures (to inject onto each new state).
struct Machine {
	children: Vec<Child>,
	root: Option<Root>,
	instantiate: Instantiator,
	bg_callback: Option<SqStrongRef>,
	bg_suspended: bool,
	methods: Vec<(&'static str, SqStrongRef)>,
}

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

/// Construct `path`, inject the state methods, register it as a child, then run its
/// `onInit` (the state shows itself from there). No machine borrow is held across
/// the script-running calls (they re-enter).
fn add_child(
	machine: &Rc<RefCell<Machine>>,
	vm: &SQVM,
	name: &str,
	path: &str,
) -> Result<(), String> {
	let instantiate = machine.borrow().instantiate.clone();
	let child = instantiate(vm, path)?;
	let id = child.identity();
	inject(machine, vm, &child);
	machine.borrow_mut().children.push(Child {
		name: name.to_string(),
		id,
		state: child.clone(),
	});
	child.call_method(vm, "onInit", ())
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
				methods: Vec::new(),
			})),
		}
	}

	/// Build the shared native state methods. Call once, with `&mut SQVM`, before
	/// [`boot`](Self::boot).
	pub fn install(&self, vm: &mut SQVM) {
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
				move |vm: &SQVM, name: String, path: String| -> Result<(), String> {
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
				let target = m.borrow().by_id(vm.this_identity());
				target.map_or(Ok(()), |t| t.call_method(vm, "onShow", ()))
			})
		};
		let hide = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let target = m.borrow().by_id(vm.this_identity());
				target.map_or(Ok(()), |t| t.call_method(vm, "onHide", ()))
			})
		};
		let finish = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let target = m.borrow_mut().remove(vm.this_identity());
				target.map_or(Ok(()), |t| t.call_method(vm, "onFinish", ()))
			})
		};
		let send_to_siblings = {
			let m = self.machine.clone();
			vm.make_closure(
				move |vm: &SQVM, msg: squirrel::Retained| -> Result<(), String> {
					let siblings = m.borrow().siblings_of(vm.this_identity());
					for sibling in siblings {
						ignore_missing(sibling.call_method(
							vm,
							"onSiblingSentMessage",
							(msg.0.clone(),),
						))?;
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

		self.machine.borrow_mut().methods = vec![
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
	/// — which builds the global UI objects and `add`s the first child state. Returns
	/// the root. Call [`install`](Self::install) first.
	pub fn boot(&self, vm: &SQVM, root_path: &str) -> Result<SqStrongRef, String> {
		let instantiate = self.machine.borrow().instantiate.clone();
		let root = instantiate(vm, root_path)?;
		let id = root.identity();
		inject(&self.machine, vm, &root);
		self.machine.borrow_mut().root = Some(Root {
			id,
			state: root.clone(),
		});
		SqTable::root(vm).set("RootState", root.clone());
		root.call_method(vm, "onInit", ())?;
		Ok(root)
	}

	// ---- host / inspection API ----

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
		machine.bg_callback = None;
		machine.methods.clear();
	}
}
