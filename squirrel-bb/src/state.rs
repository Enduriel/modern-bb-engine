//! The module system's shared mutable state.
//!
//! The native `include` / `inherit` / `new` / `isKindOf` functions are plain
//! closures registered with the VM, so they must reach this state through a
//! shared handle. Squirrel can re-enter them (running a script triggers more
//! `inherit`/`new` calls), so every access borrows the [`RefCell`] in a short
//! scope and never holds it across a VM call — see [`crate::module`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use squirrel::SqStrongRef;

use crate::source::ScriptSource;

/// A handle to the module system's state, shared between the [`ModuleSystem`] and
/// the native closures it installs into the VM.
///
/// [`ModuleSystem`]: crate::ModuleSystem
pub(crate) type SharedState<S> = Rc<RefCell<State<S>>>;

/// All persistent state the module system keeps Rust-side. Paths are stored
/// normalized (see [`crate::hash::normalize_path`]).
pub(crate) struct State<S: ScriptSource> {
	/// Resolves a script path to its source bytes (the injected VFS seam).
	pub(crate) source: S,
	/// Class prototypes by path: the table a class script registered (a base
	/// literal, or the result of `inherit`). Cloned per instance by `new`.
	pub(crate) prototypes: HashMap<String, SqStrongRef>,
	/// Scripts already compiled+run, so `include` runs each at most once.
	pub(crate) included: HashSet<String>,
	/// Each included file's own scope: the table it ran with as `this`, where its
	/// file-scope definitions (a base class's literal, its helper functions) live.
	pub(crate) scopes: HashMap<String, SqStrongRef>,
	/// Inheritance links: child path → parent path, recorded at `inherit` time so
	/// `new` can walk the chain to clone every ancestor.
	pub(crate) parents: HashMap<String, String>,
	/// `ClassNameHash` → path, the reverse half of the `IO` registry.
	pub(crate) by_hash: HashMap<i64, String>,
	/// Paths currently being loaded (a stack, since includes nest). The top is the
	/// script whose top-level code is running, which `inherit` uses as the child's
	/// own path for `ClassNameHash` stamping.
	pub(crate) load_stack: Vec<String>,
}

impl<S: ScriptSource> State<S> {
	pub(crate) fn new(source: S) -> Self {
		State {
			source,
			prototypes: HashMap::new(),
			included: HashSet::new(),
			scopes: HashMap::new(),
			parents: HashMap::new(),
			by_hash: HashMap::new(),
			load_stack: Vec::new(),
		}
	}
}
