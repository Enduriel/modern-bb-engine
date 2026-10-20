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
//! The prototypes are laid out the same way (probed 2026-10-14): `inherit` gives each
//! class its **own copy** of its ancestors' levels, linked like an instance's, in its
//! super slot, and the class's own table has no delegate. So a hook framework that
//! writes a field through a class's ancestor levels (Modern Hooks' `q.m.X = v` finds
//! the level that declares `X` by walking `p[p.SuperName]`) changes that class (and
//! classes inheriting from it later) only: Reforged sets `BaseItemScript` on one named
//! weapon this way. `new` builds an instance from the leaf's own chain.
//!
//! **Lazy levels.** Every level table above, an instance's and a class's
//! ancestor copy alike, is a *lazy clone* of the level it copies (`SqTable::lazy_clone`,
//! VM support in squirrel-sys `sqtable.cpp`): it behaves exactly as the plain clone (and,
//! for an instance's ancestor levels, the binding of every closure to the leaf) would,
//! but holds only what was read or written. An inherited method is bound to the leaf
//! when first read and kept, so it is the same object on every read and differs per
//! instance, as in the original. Iterating a level, cloning it or deleting a slot first
//! turns it into the plain table, laid out slot for slot as the clone would be (so
//! `foreach` order is the same), and a write to a prototype after instances were made
//! from it isn't seen by them (they keep a copy of it as it was). With a large mod
//! corpus and a day-5 save this took the VM from ~466 to ~240 MB; `BB_LAZY_LEVELS=0` builds
//! plain clones, to compare.
//!
//! Re-entrancy: running a script (or a constructor) re-enters these functions, so
//! every [`SharedState`] borrow is taken in a short scope and never held across a
//! VM call.

use squirrel::{CensusTrack, SQVM, SqStrongRef, SqTable};

use crate::error::ScriptError;
use crate::hash::{basename, class_name_hash, normalize_path};
use crate::module::include;
use crate::source::ScriptSource;
use crate::state::SharedState;

/// Resolve the class prototype for `path`, loading the script if needed.
///
/// A derived class is registered by [`inherit`] while its script runs; a base
/// class is a plain literal under its basename in the file's own scope (see
/// `include`), which we adopt (stamping `ClassNameHash`) the first time it is
/// requested.
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

	// Base class: a plain literal under its basename in the file's scope. Adopt it.
	let scope = state.borrow().scopes.get(&norm).cloned();
	let mut proto = scope
		.as_ref()
		.and_then(|s| s.as_table(vm))
		.and_then(|s| s.get::<SqTable>(basename(&norm)))
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
	let lazy = state.borrow().lazy_levels;
	let parent_levels = copy_levels(vm, &parent, lazy);
	let parent_norm = normalize_path(parent_path);

	// The child's own path is the script currently being loaded.
	let child_path = state.borrow().load_stack.last().cloned();

	// The prototype keeps only the fields it declares in its own `m`, and no delegate
	// (like the original); `new` links an instance's levels.
	ensure_m(vm, &mut body);

	// Explicit super-calls go through a slot named after the parent class
	// (`this.<parent>.create()`), recorded in `SuperName` (which hook frameworks walk:
	// `p = p[p.SuperName]`). It holds this class's own copy of the parent's levels.
	body.set("SuperName", basename(&parent_norm).to_string());
	body.set(basename(&parent_norm), parent_levels);

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
	let leaf = prototype(vm, state, &norm)?;

	// The layout is the engine's work, not the scripts': a census tracks what scripts
	// touch from `create()` on.
	vm.census_track(CensusTrack::Pause);
	// The class's own chain, leaf → … → base, through its super slots.
	let mut chain = vec![leaf];
	while let Some(parent) = super_level(chain.last().expect("the chain is never empty")) {
		chain.push(parent);
	}

	// One table per class level, base → leaf: a shallow clone of the level's
	// prototype (its own methods + metadata), with its own deep-cloned `m` whose
	// delegate is the parent level's `m`, and the parent level as both delegate and
	// super-slot.
	let lazy = state.borrow().lazy_levels;
	let mut levels: Vec<SqTable> = Vec::with_capacity(chain.len());
	for proto in chain.iter().rev() {
		let mut table = if lazy { proto.lazy_clone() } else { proto.shallow_clone() };
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
				let parent_name = proto.get_owned::<String>("SuperName").expect("a level with a parent names it");
				table.set(&parent_name, parent.to_strong());
			}
			None => table.clear_delegate(),
		}
		table.set("m", m);
		levels.push(table);
	}

	// Bind the ancestor levels' methods to the leaf, so `this` is the full instance even
	// in super-calls. The leaf's own methods stay the class's closures, shared by every
	// instance and unbound, as in the original (parity probe `menu/class_layout`).
	let leaf_table = levels.last().expect("the chain is never empty").to_strong();
	let leaf_table = leaf_table.as_table(vm).expect("leaf is a table");
	let ancestors = levels.len() - 1;
	for table in &mut levels[..ancestors] {
		if lazy {
			table.lazy_bind_to(&leaf_table);
		} else {
			table.bind_closures_to(&leaf_table);
		}
	}

	// What else the original puts on an instance (parity probe `menu/class_layout`):
	// on the base level, `InstanceName` ("<class>@<n>", counting the class's instances)
	// and the `self`/`base` natives bound to the instance; on the leaf, a userdata (its
	// `_release_hook_DO_NOT_delete_it_`, an opaque token here).
	let (count, natives) = {
		let mut st = state.borrow_mut();
		let count = st.instance_counts.entry(norm.clone()).or_insert(0);
		*count += 1;
		(*count, st.class_natives.clone())
	};
	{
		// The base level has no delegate, so `contains` only sees its own slots.
		let base_level = &mut levels[0];
		base_level.set("InstanceName", format!("{}@{count}", basename(&norm)));
		for (name, native) in natives.iter().flat_map(|(s, b)| [("self", s), ("base", b)]) {
			if !base_level.contains(name) {
				if let Some(bound) = native.bindenv(vm, &leaf_table) {
					base_level.set(name, bound);
				}
			}
		}
	}
	levels
		.last_mut()
		.expect("the chain is never empty")
		.set("_release_hook_DO_NOT_delete_it_", SqStrongRef::userdata(vm));
	let prev = levels.pop();

	let mut instance = prev.expect("the chain is never empty");
	instance.set("ClassNameHash", class_name_hash(&norm));
	vm.census_track(CensusTrack::Resume);
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

/// The level above `level` in its class's chain: its super slot, named by `SuperName`.
fn super_level<'vm>(level: &SqTable<'vm>) -> Option<SqTable<'vm>> {
	let name = level.get_owned::<String>("SuperName")?;
	level.get_owned::<SqTable>(&name)
}

/// A copy of the class level `level` and every level above it, for a derived class's
/// super slot: each level a shallow copy with its own deep-copied `m`, delegating to the
/// copy of its parent level (and its `m` to that level's `m`), as the original links
/// them (probed: `del=parent`, `m` delegating to the parent's `m`).
fn copy_levels<'vm>(vm: &'vm SQVM, level: &SqTable<'vm>, lazy: bool) -> SqStrongRef {
	let mut copy = if lazy { level.lazy_clone() } else { level.shallow_clone() };
	let mut m = match level.get_owned::<SqTable>("m") {
		Some(own) => own.deep_clone(),
		None => SqTable::new(vm),
	};
	let parent = level.get_owned::<String>("SuperName").and_then(|name| Some((level.get_owned::<SqTable>(&name)?, name)));
	match parent {
		Some((parent, name)) => {
			let parent_copy = copy_levels(vm, &parent, lazy);
			let parent_copy = parent_copy.as_table(vm).expect("a level is a table");
			if let Some(parent_m) = parent_copy.get_owned::<SqTable>("m") {
				m.set_delegate(&parent_m);
			}
			copy.set_delegate(&parent_copy);
			copy.set(&name, parent_copy.to_strong());
		}
		None => copy.clear_delegate(),
	}
	copy.set("m", m);
	copy.to_strong()
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
