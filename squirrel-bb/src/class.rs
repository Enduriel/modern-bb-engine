//! The BB prototype/class machinery: resolving prototypes, `inherit`, `new`, and
//! `isKindOf`.
//!
//! BB scripts do not use Squirrel's native `class`. A class script registers a
//! plain table in the root under its basename — either a literal (a base class)
//! or the result of `this.inherit(parentPath, body)` (a derived class). The
//! structure these functions build, for an instance of a derived class:
//!
//! ```text
//!   instance ── delegate ─▶ parentClone ── delegate ─▶ … ─▶ baseClone
//!     │  .m = sharedM           │  .m = sharedM              │  .m = sharedM
//!     │  ["parent"] ───────────▶┘  ["grandparent"] ─────────▶┘
//! ```
//!
//! * Every level shares one member table (`sharedM`), so `this.m.X` written by any
//!   ancestor's `create()` lands on the same instance state.
//! * Method lookups fall through the **delegate** chain (so inherited methods
//!   resolve), and explicit super-calls go through a slot named after the parent
//!   class (`this.actor.create()`), which points at the same per-level clone.
//! * `create()` is invoked on the leaf; each level's `create` chains up by calling
//!   `this.<parent>.create()`.
//!
//! Re-entrancy: running a script (or a constructor) re-enters these functions, so
//! every [`SharedState`] borrow is taken in a short scope and never held across a
//! VM call.

use squirrel::{SQVM, SqStrongRef, SqTable};

use crate::error::ScriptError;
use crate::hash::{basename, class_name_hash, normalize_path};
use crate::module::include;
use crate::source::ScriptSource;
use crate::state::SharedState;

/// Resolve the class prototype for `path`, loading the script if needed.
///
/// A derived class is registered by [`inherit`] while its script runs; a base
/// class is a plain root-table literal under its basename, which we adopt
/// (stamping `ClassNameHash`) the first time it is requested.
pub(crate) fn prototype<'vm, S: ScriptSource>(
	vm: &'vm SQVM,
	state: &SharedState<S>,
	path: &str,
) -> Result<SqTable<'vm>, ScriptError> {
	let norm = normalize_path(path);

	// Already cached?
	{
		let st = state.borrow();
		if let Some(proto) = st.prototypes.get(&norm) {
			return Ok(proto.as_table(vm).expect("cached prototype is a table"));
		}
	}

	// Load it. A derived class registers its prototype via `inherit` during this.
	include(vm, state, &norm)?;
	{
		let st = state.borrow();
		if let Some(proto) = st.prototypes.get(&norm) {
			return Ok(proto.as_table(vm).expect("registered prototype is a table"));
		}
	}

	// Base class: a plain literal in the root under its basename. Adopt it.
	let mut proto = SqTable::root(vm)
		.get::<SqTable>(basename(&norm))
		.ok_or_else(|| {
			ScriptError::Runtime(format!(
				"script '{norm}' did not register a class named '{}'",
				basename(&norm)
			))
		})?;
	let hash = class_name_hash(&norm);
	proto.set("ClassNameHash", hash);
	{
		let mut st = state.borrow_mut();
		st.prototypes.insert(norm.clone(), proto.to_strong());
		st.by_hash.insert(hash, norm);
	}
	Ok(proto)
}

/// `this.inherit(parentPath, body)`: turn `body` into a derived-class prototype of
/// the class at `parentPath`, and return it (the script slots it into the root).
pub(crate) fn inherit<S: ScriptSource>(
	vm: &SQVM,
	state: &SharedState<S>,
	parent_path: &str,
	mut body: SqTable,
) -> Result<SqStrongRef, ScriptError> {
	let parent = prototype(vm, state, parent_path)?;
	let parent_norm = normalize_path(parent_path);

	// The child's own path is the script currently being loaded.
	let child_path = state.borrow().load_stack.last().cloned();

	// Layer the parent's (already-complete) member defaults beneath the child's,
	// so this prototype's `m` carries every inherited default.
	if let Some(parent_m) = parent.get::<SqTable>("m") {
		let mut merged = parent_m.deep_clone();
		if let Some(body_m) = body.get::<SqTable>("m") {
			merged.deep_merge_from(&body_m);
		}
		body.set("m", merged);
	}

	// Method inheritance via the delegate; explicit super-calls via a slot named
	// after the parent class (`this.<parent>.create()`). Both reference the parent
	// prototype here — `new` re-points them at per-instance clones.
	body.set_delegate(&parent);
	body.set(basename(&parent_norm), parent);

	if let Some(child) = child_path {
		let hash = class_name_hash(&child);
		body.set("ClassNameHash", hash);
		let mut st = state.borrow_mut();
		st.prototypes.insert(child.clone(), body.to_strong());
		st.parents.insert(child.clone(), parent_norm);
		st.by_hash.insert(hash, child);
	}

	Ok(body.to_strong())
}

/// `this.new(path)`: instantiate the class at `path` — clone its prototype chain
/// into a fresh object with its own member table, then run the `create()` chain.
pub(crate) fn new_instance<S: ScriptSource>(
	vm: &SQVM,
	state: &SharedState<S>,
	path: &str,
) -> Result<SqStrongRef, ScriptError> {
	let norm = normalize_path(path);
	let leaf = prototype(vm, state, &norm)?;

	// The chain leaf → … → base, via the recorded parent links.
	let chain: Vec<String> = {
		let st = state.borrow();
		let mut chain = vec![norm.clone()];
		let mut cur = norm.clone();
		while let Some(parent) = st.parents.get(&cur) {
			chain.push(parent.clone());
			cur = parent.clone();
		}
		chain
	};

	// One member table for the whole instance, deep-cloned from the leaf's
	// already-merged defaults so instances never share mutable state.
	let shared_m = match leaf.get::<SqTable>("m") {
		Some(m) => m.deep_clone(),
		None => SqTable::new(vm),
	}
	.to_strong();

	// Clone base → leaf, sharing `m` and wiring each level's delegate + super-ref.
	let mut prev: Option<SqTable> = None;
	let mut prev_name: Option<String> = None;
	for level in chain.iter().rev() {
		let proto = {
			let st = state.borrow();
			st.prototypes
				.get(level)
				.expect("every prototype in the chain is cached")
				.as_table(vm)
				.expect("prototype is a table")
		};
		let mut clone = proto.shallow_clone();
		clone.set("m", shared_m.as_table(vm).expect("shared m is a table"));
		if let (Some(parent_clone), Some(name)) = (prev.take(), prev_name.take()) {
			clone.set_delegate(&parent_clone);
			clone.set(&name, parent_clone);
		}
		prev = Some(clone);
		prev_name = Some(basename(level).to_string());
	}

	let mut instance = prev.expect("the chain is never empty");
	instance.set("ClassNameHash", class_name_hash(&norm));
	instance
		.call_method("create", ())
		.map_err(ScriptError::Runtime)?;
	Ok(instance.to_strong())
}

/// `isKindOf(obj, className)`: whether `obj` is an instance of the class named
/// `className` (a basename) or any ancestor — by walking the delegate chain and
/// reverse-mapping each level's `ClassNameHash` back to its path.
pub(crate) fn is_kind_of<S: ScriptSource>(
	state: &SharedState<S>,
	obj: SqTable,
	class_name: &str,
) -> bool {
	let mut cur = Some(obj);
	while let Some(level) = cur {
		if let Some(hash) = level.get::<i64>("ClassNameHash") {
			let matched = {
				let st = state.borrow();
				st.by_hash
					.get(&hash)
					.is_some_and(|p| basename(p) == class_name)
			};
			if matched {
				return true;
			}
		}
		cur = level.delegate();
	}
	false
}
