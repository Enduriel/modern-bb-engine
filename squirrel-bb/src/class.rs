//! The BB prototype/class machinery: resolving prototypes, `inherit`, `new`, and
//! `isKindOf`.
//!
//! BB scripts do not use Squirrel's native `class`. A class script registers a
//! plain table in the root under its basename — either a literal (a base class)
//! or the result of `this.inherit(parentPath, body)` (a derived class). The
//! structure these functions build, for an instance of a derived class:
//!
//! The layout matches the ORIGINAL engine (probed in the real game). For an instance of `brave_trait` →
//! `character_trait` → `skill`:
//!
//! ```text
//!   leaf ──delegate──▶ T_character_trait ──delegate──▶ T_skill ──▶ (none)
//!    │ .character_trait ─▶┘   │ .skill ──────────────▶┘
//!    │ .m ──delegate──▶ .m ──delegate──▶ .m      (each level's OWN fields)
//! ```
//!
//! * Every class level is a **per-instance table** holding that level's own methods
//!   (plus `ClassName`/`ClassNameHash`/`SuperName`), reached from the level below by
//!   delegate (method inheritance) and by a slot named after the parent class
//!   (explicit super-calls: `this.actor.onInit()`).
//! * Each level has its **own `m`** holding only the fields that level declares;
//!   the `m` tables are delegate-chained the same way. Reads fall through, and
//!   Squirrel's assignment fallthrough writes an inherited field where it lives, so
//!   `this.m.X = v` from any level updates the one instance state.
//! * Every closure on every level is **bound to the leaf** (`bindenv`), so a
//!   super-call still runs with `this` = the full instance: `setActor(this)` inside
//!   `actor.onInit` stores the leaf, and derived-only methods stay reachable.
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
	proto.set("ClassName", basename(&norm).to_string());
	ensure_m(vm, &mut proto);
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

	// The prototype keeps only the fields it declares in its own `m` (like the
	// original); `new` chains the per-level `m` tables by delegate.
	ensure_m(vm, &mut body);

	// Method inheritance via the delegate; explicit super-calls via a slot named
	// after the parent class (`this.<parent>.create()`), recorded in `SuperName`
	// (which hook frameworks walk: `p = p[p.SuperName]`). Both reference the parent
	// prototype here — `new` re-points them at per-instance level tables.
	body.set_delegate(&parent);
	body.set("SuperName", basename(&parent_norm).to_string());
	body.set(basename(&parent_norm), parent);

	if let Some(child) = child_path {
		let hash = class_name_hash(&child);
		body.set("ClassNameHash", hash);
		body.set("ClassName", basename(&child).to_string());
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
	// Load the class (and, through `inherit`, its whole chain).
	prototype(vm, state, &norm)?;

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

	// One table per class level, base → leaf: a shallow clone of the level's
	// prototype (its own methods + metadata), with its own deep-cloned `m` whose
	// delegate is the parent level's `m`, and the parent level as both delegate and
	// super-slot.
	let mut levels: Vec<SqTable> = Vec::with_capacity(chain.len());
	for level in chain.iter().rev() {
		let proto = {
			let st = state.borrow();
			st.prototypes
				.get(level)
				.expect("every prototype in the chain is cached")
				.as_table(vm)
				.expect("prototype is a table")
		};
		let mut table = proto.shallow_clone();
		let mut m = match proto.get_owned::<SqTable>("m") {
			Some(own) => own.deep_clone(),
			None => SqTable::new(vm),
		};
		match levels.last() {
			Some(parent) => {
				if let Some(parent_m) = parent.get::<SqTable>("m") {
					m.set_delegate(&parent_m);
				}
				table.set_delegate(parent);
				let parent_name = basename(chain_parent(&chain, level)).to_string();
				table.set(&parent_name, parent.to_strong());
			}
			None => table.clear_delegate(),
		}
		table.set("m", m);
		levels.push(table);
	}

	// Bind every level's methods to the leaf, so `this` is the full instance even in
	// super-calls.
	let leaf_table = levels.last().expect("the chain is never empty").to_strong();
	let leaf_table = leaf_table.as_table(vm).expect("leaf is a table");
	for table in &mut levels {
		table.bind_closures_to(&leaf_table);
	}
	let prev = levels.pop();

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

/// The parent path of `level` within `chain` (leaf → base order).
fn chain_parent<'a>(chain: &'a [String], level: &str) -> &'a str {
	let i = chain
		.iter()
		.position(|p| p == level)
		.expect("level is in its own chain");
	&chain[i + 1]
}

/// Give a prototype an (empty) `m` if its script declared none, so every level's
/// member table can join the instance's `m` delegate chain.
fn ensure_m(vm: &SQVM, proto: &mut SqTable) {
	if proto.get_owned::<SqTable>("m").is_none() {
		proto.set("m", SqTable::new(vm));
	}
}

/// An instance's class-level tables, leaf first, base last (the delegate chain a
/// BB instance is built from). Hosts that inject native methods use this to reach
/// the base level and to register every level's identity.
pub(crate) fn instance_levels<'vm>(leaf: SqTable<'vm>) -> Vec<SqTable<'vm>> {
	let mut levels = Vec::new();
	let mut cur = Some(leaf);
	while let Some(level) = cur {
		cur = level.delegate();
		levels.push(level);
	}
	levels
}
