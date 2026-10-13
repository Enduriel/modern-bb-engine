//! The [`ModuleSystem`]: the public handle, the `include` loader, and installing
//! the native `include` / `inherit` / `new` / `isKindOf` functions plus the `IO`
//! hash registry into a VM.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Retained, SQVM, SqArray, SqStrongRef, SqTable};

use crate::class::{inherit, is_kind_of, new_instance, prototype};
use crate::compile_context::compile_context;
use crate::error::ScriptError;
use crate::hash::{class_name_hash, normalize_path};
use crate::source::{LoadedScript, ScriptSource};
use crate::state::{SharedState, State};

/// Load, compile and run the script at `path` exactly once.
///
/// The path is marked loaded (and pushed as the current load path) *before* the
/// script runs, so a class script's top-level `inherit` sees its own path and a
/// circular include cannot loop. No state borrow is held while the script runs,
/// since it re-enters the module system.
pub(crate) fn include<S: ScriptSource>(
	vm: &SQVM,
	state: &SharedState<S>,
	path: &str,
) -> Result<(), ScriptError> {
	let norm = normalize_path(path);

	if state.borrow().included.contains(&norm) {
		return Ok(());
	}

	let caches_compiles = state.borrow().source.caches_compiles();
	let script = state
		.borrow()
		.source
		.load_for(&norm, &|| compile_context(vm))
		.ok_or_else(|| ScriptError::NotFound(norm.clone()))?;

	{
		let mut st = state.borrow_mut();
		st.included.insert(norm.clone());
		st.load_stack.push(norm.clone());
	}

	// Source is compiled, bytecode loaded; either way the closure runs with `this` =
	// root. Game (and mod) scripts compute floats in single precision like the
	// original 32-bit-float engine — required for seed-exact worldgen (see
	// `SqStrongRef::set_f32_math`). No state borrow is held across the run.
	let closure = match script {
		LoadedScript::Source(bytes) => {
			let before = caches_compiles.then(|| compile_context(vm));
			let compiled = vm.compile(&String::from_utf8_lossy(&bytes), &norm);
			// Offer the compile for caching, unless it declared constants: loading it
			// from a cache would skip those declarations, which later compiles rely on.
			if let (Ok(closure), Some(before)) = (&compiled, before)
				&& compile_context(vm) == before
				&& let Ok(bytecode) = closure.write_closure(vm)
			{
				state.borrow().source.compiled(&norm, before, &bytecode);
			}
			compiled
		}
		LoadedScript::Bytecode(bytes) => vm.load_closure(&bytes),
	};
	let result = closure.and_then(|closure| {
		closure.set_f32_math(vm, true);
		closure.call(vm, ())
	});

	state.borrow_mut().load_stack.pop();
	result.map_err(ScriptError::Runtime)
}

/// A `ScriptSource`-erased "instantiate a class by path" callback (the `this.new`
/// semantics), so a subsystem can construct script objects without naming the
/// concrete [`ScriptSource`] type — e.g. the state stack creating its states. Get
/// one from [`ModuleSystem::instantiator`]; it shares the same prototype cache.
pub type Instantiator = Rc<dyn Fn(&SQVM, &str) -> Result<SqStrongRef, String>>;

/// Reads a non-script game file through the [`ScriptSource`] (see
/// [`ScriptSource::read_file`]).
pub type FileReader = Rc<dyn Fn(&str) -> Option<Vec<u8>>>;

/// The Battle Brothers module / class system for a VM.
///
/// Construct one with a [`ScriptSource`], [`install`](Self::install) its native
/// functions into the VM (before any game script runs), then drive loading with
/// [`include`](Self::include) / [`instantiate`](Self::instantiate). Game scripts
/// use `this.include` / `this.inherit` / `this.new` / `isKindOf` directly.
///
/// The state is shared (an `Rc<RefCell<…>>`) with the installed closures, so the
/// `ModuleSystem` handle and the VM's native functions operate on the same cache.
///
/// # Drop order
/// The prototype cache holds [`SqStrongRef`]s, which release against the VM on
/// drop, so a `ModuleSystem` must be dropped **before** its VM is closed (the host
/// owns the VM at the top and drops the service first, which satisfies this).
pub struct ModuleSystem<S: ScriptSource> {
	state: SharedState<S>,
}

impl<S: ScriptSource + 'static> ModuleSystem<S> {
	/// Create a module system backed by `source` for resolving script paths.
	pub fn new(source: S) -> Self {
		ModuleSystem {
			state: Rc::new(RefCell::new(State::new(source))),
		}
	}

	/// Install the native module-system functions into `vm`'s root table:
	/// `include`, `inherit`, `new`, `isKindOf`, and the `IO` hash registry
	/// (`scriptFilenameByHash` / `scriptHashByFilename`). Call once, before any
	/// game script runs.
	pub fn install(&self, vm: &mut SQVM) {
		{
			let state = self.state.clone();
			vm.register("include", move |vm: &SQVM, path: &str| {
				include(vm, &state, path)
			});
		}
		{
			let state = self.state.clone();
			vm.register("inherit", move |vm: &SQVM, parent: &str, body: SqTable| {
				inherit(vm, &state, parent, body)
			});
		}
		{
			let state = self.state.clone();
			vm.register("new", move |vm: &SQVM, path: &str| {
				new_instance(vm, &state, path)
			});
		}
		{
			let state = self.state.clone();
			// `isKindOf` must accept ANY value for the object (BB calls e.g.
			// `isKindOf("trait.oath_of_honor", "skill")` with a plain string and
			// expects `false`, not a type error). Take it as a `Retained` (typemask
			// `.`), and only walk the chain when it is actually a table/instance.
			vm.register("isKindOf", move |vm: &SQVM, obj: Retained, name: &str| {
				obj.0
					.as_table(vm)
					.is_some_and(|t| is_kind_of(&state, t, name))
			});
		}
		self.install_io(vm);
	}

	/// Build the `IO` table (`scriptFilenameByHash` / `scriptHashByFilename` /
	/// `enumerateFiles`) and slot it into the root.
	fn install_io(&self, vm: &mut SQVM) {
		let by_hash = {
			let state = self.state.clone();
			vm.make_closure(move |hash: i64| -> Option<String> {
				state.borrow().by_hash.get(&hash).cloned()
			})
		};
		let by_name = {
			let state = self.state.clone();
			vm.make_closure(move |path: &str| -> Option<i64> {
				// Like the original: only an existing script, named by its exact
				// canonical path, has a hash; anything else (wrong case, a `.nut`
				// suffix, an unknown path) yields null. Remember the mapping so a later
				// `scriptFilenameByHash` can reverse it even for an unloaded script.
				if path != normalize_path(path) {
					return None;
				}
				let known = state.borrow().prototypes.contains_key(path)
					|| state.borrow().source.load(path).is_some();
				if !known {
					return None;
				}
				let hash = class_name_hash(path);
				state
					.borrow_mut()
					.by_hash
					.entry(hash)
					.or_insert_with(|| path.to_string());
				Some(hash)
			})
		};
		let enumerate = {
			let state = self.state.clone();
			vm.make_closure(move |vm: &SQVM, prefix: &str| -> Option<SqStrongRef> {
				// `None` (a missing directory) becomes Squirrel `null`, which
				// `preload.nut`'s DLC sweep guards against. Otherwise return an array
				// of `include`-able paths.
				let paths = state.borrow().source.list_prefix(prefix)?;
				let mut files = SqArray::new(vm);
				for path in paths {
					files.push(path);
				}
				Some(files.to_strong())
			})
		};

		let mut io = SqTable::new(vm);
		io.set("scriptFilenameByHash", by_hash);
		io.set("scriptHashByFilename", by_name);
		io.set("enumerateFiles", enumerate);
		SqTable::root(vm).set("IO", io);
	}

	/// Load, compile and run the script at `path` (once). Host entry point for the
	/// `this.include` semantics — e.g. running `preload`.
	///
	/// Drive the boot through this (`include("scripts/preload")`), not a bare
	/// `run_script`: `preload`'s final `enumerateFiles("scripts/")` sweep lists
	/// `scripts/preload` itself, and only the run-once guard (which marks a path
	/// loaded *before* running it) makes that self-include a harmless no-op rather
	/// than unbounded recursion.
	pub fn include(&self, vm: &SQVM, path: &str) -> Result<(), ScriptError> {
		include(vm, &self.state, path)
	}

	/// Instantiate the class at `path` (the `this.new` semantics), returning the
	/// constructed object as an owned strong reference for the host to keep.
	pub fn instantiate(&self, vm: &SQVM, path: &str) -> Result<SqStrongRef, ScriptError> {
		new_instance(vm, &self.state, path)
	}

	/// A clonable, `ScriptSource`-erased [`Instantiator`] sharing this system's
	/// prototype cache — for subsystems (e.g. the state stack) that construct script
	/// objects but must not be generic over the source type.
	pub fn instantiator(&self) -> Instantiator {
		let state = self.state.clone();
		Rc::new(move |vm: &SQVM, path: &str| {
			new_instance(vm, &state, path).map_err(|e| e.to_string())
		})
	}

	/// A handle that reads non-script game files from the source.
	pub fn file_reader(&self) -> FileReader {
		let state = self.state.clone();
		Rc::new(move |path: &str| state.borrow().source.read_file(path))
	}

	/// Resolve (loading if needed) the class prototype for `path` as an owned
	/// strong reference.
	pub fn prototype(&self, vm: &SQVM, path: &str) -> Result<SqStrongRef, ScriptError> {
		Ok(prototype(vm, &self.state, path)?.to_strong())
	}

	/// Whether the script at `path` has already been loaded.
	pub fn is_loaded(&self, path: &str) -> bool {
		self.state.borrow().included.contains(&normalize_path(path))
	}
}

impl<S: ScriptSource> ModuleSystem<S> {
	/// Release the prototype cache (its [`SqStrongRef`]s). **Call this before the VM
	/// is closed.**
	///
	/// Dropping the `ModuleSystem` handle is NOT enough: the native `include`/
	/// `inherit`/`new` closures (and any subsystem holding the
	/// [`instantiator`](Self::instantiator), e.g. the state stack) capture the shared
	/// `State`, so it — and the prototype `SqStrongRef`s it holds — stays alive until
	/// those VM closures are freed *during* `sq_close`. Releasing prototype refs at
	/// that point is a use-after-free (the objects are mid-teardown). Clearing here,
	/// while the VM is fully alive, releases them safely; the later (empty) `State`
	/// drop during `sq_close` is then a no-op. (Not bound by `'static` so [`Drop`]
	/// impls — e.g. [`Engine`](crate::Engine) — can call it.)
	pub fn clear(&self) {
		let mut st = self.state.borrow_mut();
		st.prototypes.clear();
		st.parents.clear();
		st.by_hash.clear();
		st.included.clear();
	}
}
