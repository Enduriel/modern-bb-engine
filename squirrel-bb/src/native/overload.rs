//! Sqrat-style overloaded native functions.
//!
//! Battle Brothers' C++ bindings (Sqrat) expose an overloaded function `name` as a
//! *dispatcher* plus one leaf per arity, slotted under `"__sqrat_ol_ <name>_<N>"`
//! (the space after the prefix is significant — modern_hooks discovers overloads with
//! `regexp("^__sqrat_ol_ \\w+_(\\d+)$")`). Calling `Math.rand()` runs
//! `Math["__sqrat_ol_ rand_0"]()`; `Math.rand(0, 5)` runs `Math["__sqrat_ol_ rand_2"]`.
//!
//! Reproducing this exactly (verified against the original engine) buys two things:
//! the leaves are real, named, replaceable slots, so a mod can wrap *one* overload;
//! and the dispatcher re-resolves the leaf on the live receiver every call, so that
//! replacement is honoured. The original dispatcher is **strict** — an arity with no
//! registered leaf raises `"No overload matching this argument list found"` rather
//! than guessing — so we match that (the leaves themselves are exact-arity too, which
//! the project deliberately keeps strict).
//!
//! ## Wiring
//!
//! Register the leaves under [`overload_key`] (on a table via `SqTable::set`, or on a
//! class via `ClassBuilder::method`), then add the dispatcher under the bare name —
//! on a table with [`make_dispatcher`], on a class with `ClassBuilder::native_method`
//! and an [`Overload`]: `let d = Overload::new("<name>");` then
//! `move |vm, this, args| d.dispatch(vm, this, args)`.

use std::cell::OnceCell;
use std::io::Write;

use squirrel::{Forwarded, SQVM, SqStrongRef, StackArgs, This};

/// The exact slot key for the arity-`arity` leaf of overloaded native `base_name`:
/// `"__sqrat_ol_ <base_name>_<arity>"`. The single source of truth for the key
/// format, shared by registration ([`overload_key`]) and dispatch ([`Overload`]).
pub(crate) fn overload_key(base_name: &str, arity: usize) -> String {
	format!("__sqrat_ol_ {base_name}_{arity}")
}

/// Run `f` with [`overload_key`]`(base_name, arity)`, built in a stack buffer when it
/// fits (dispatch runs on every overloaded call, so it avoids a heap allocation).
fn with_overload_key<R>(base_name: &str, arity: usize, f: impl FnOnce(&str) -> R) -> R {
	let mut buf = [0u8; 96];
	let mut rest = &mut buf[..];
	if write!(rest, "__sqrat_ol_ {base_name}_{arity}").is_ok() {
		let len = 96 - rest.len();
		// Only `str` pieces were written, so the bytes are valid UTF-8.
		f(std::str::from_utf8(&buf[..len]).expect("formatted from str"))
	} else {
		f(&overload_key(base_name, arity))
	}
}

/// The arities whose leaf key an [`Overload`] keeps (Sqrat's go up to 9 arguments).
const CACHED_ARITIES: usize = 12;

/// A Sqrat overload dispatcher: count the call's args `N` (the receiver excluded),
/// resolve `"__sqrat_ol_ <base_name>_<N>"` on the *live* receiver, and call it with
/// `this` bound, forwarding the args unchanged. Re-resolving on `this` each call is what
/// makes a mod's replacement of a specific overload take effect. The args are forwarded
/// from the call's own stack slots and the leaf's result is returned in place, so no
/// references are taken, and each arity's key is interned once (on its first call), so
/// a call doesn't format or hash it.
///
/// Strict like the original engine: an arity with no matching leaf raises the engine's
/// exact error string.
pub(crate) struct Overload {
	base_name: String,
	/// `OnceCell`s, not a `RefCell`: a leaf may call the same overloaded native again.
	keys: [OnceCell<SqStrongRef>; CACHED_ARITIES],
}

impl Overload {
	pub(crate) fn new(base_name: &str) -> Self {
		Overload {
			base_name: base_name.to_string(),
			keys: Default::default(),
		}
	}

	pub(crate) fn dispatch(&self, vm: &SQVM, this: This, args: StackArgs) -> Result<Forwarded, String> {
		let arity = args.len();
		let found = match self.keys.get(arity) {
			Some(cell) => {
				let key = cell.get_or_init(|| SqStrongRef::string(vm, &overload_key(&self.base_name, arity)));
				this.forward_member(vm, key, args)?
			}
			None => with_overload_key(&self.base_name, arity, |key| this.forward_member(vm, key, args))?,
		};
		found.ok_or_else(|| "No overload matching this argument list found".to_string())
	}
}

/// Build the dispatcher closure for an overloaded native named `base_name`, ready to
/// slot under the bare name on a **table** (e.g. `Math.rand`). For an overloaded
/// **class method**, use `ClassBuilder::native_method` with an inline adapter instead
/// (the dispatcher must be a class member so `inst.name(...)` resolves to it).
pub(crate) fn make_dispatcher(vm: &mut SQVM, base_name: &str) -> SqStrongRef {
	let overload = Overload::new(base_name);
	vm.make_closure(move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args))
}

/// The parameter check of a leaf that takes its arguments as a `Variadic`: the dispatcher
/// only picks it for exactly `arity` arguments, and a direct call through its slot with
/// another count fails as a native's own parameter check would.
pub(crate) fn expect_arity(args: usize, arity: usize) -> Result<(), String> {
	if args == arity { Ok(()) } else { Err("wrong number of parameters".to_string()) }
}

/// Bind `$body` as class method `$name`, overloaded as the original's Sqrat bindings have
/// it: a leaf per arity in `[$n, …]` under [`overload_key`] (each refusing another count,
/// see [`expect_arity`]) and the strict dispatcher under `$name`. `$body` is
/// `|payload: &mut $T, vm, args: &[Retained]| -> Result<_, String>`; the leaves share it.
/// (A macro: a generic function couldn't name the leaves' return-value bound.)
macro_rules! class_overloads {
	($b:expr, $T:ty, $name:expr, [$($n:expr),+], $body:expr) => {{
		let body = std::rc::Rc::new($body);
		let b = $b;
		$(
			let leaf = body.clone();
			let b = b.method(
				&$crate::native::overload::overload_key($name, $n),
				move |t: &mut $T, vm: &squirrel::SQVM, squirrel::Variadic(a): squirrel::Variadic<squirrel::Retained>| {
					$crate::native::overload::expect_arity(a.len(), $n)?;
					(*leaf)(t, vm, &a)
				},
			);
		)+
		let dispatcher = $crate::native::overload::Overload::new($name);
		b.native_method($name, move |vm: &squirrel::SQVM, this: squirrel::This, args: squirrel::StackArgs| {
			dispatcher.dispatch(vm, this, args)
		})
	}};
}
pub(crate) use class_overloads;

/// The two slots of a native method the original binds as a Sqrat overload with a single
/// arity (e.g. `storeSpriteColors`, only `"__sqrat_ol_ storeSpriteColors_0"`): the leaf
/// under its key and the dispatcher under the bare name, for the `(name, closure)` method
/// lists injected onto script objects. `key` is [`overload_key`]`(name, arity)` spelled
/// out, since those lists hold `&'static str`s.
pub(crate) fn single_overload(
	vm: &mut SQVM,
	name: &'static str,
	key: &'static str,
	leaf: SqStrongRef,
) -> [(&'static str, SqStrongRef); 2] {
	debug_assert!(
		key.strip_prefix("__sqrat_ol_ ")
			.and_then(|k| k.strip_prefix(name))
			.and_then(|k| k.strip_prefix('_'))
			.is_some_and(|n| n.parse::<usize>().is_ok()),
		"{key} is not an overload slot of {name}"
	);
	[(key, leaf), (name, make_dispatcher(vm, name))]
}
