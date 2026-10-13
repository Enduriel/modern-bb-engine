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
//! and a `|vm, this, args| dispatch_overload(vm, this, "<name>", args)` adapter.

use std::io::Write;

use squirrel::{Forwarded, SQVM, SqStrongRef, StackArg, This, Variadic};

/// The exact slot key for the arity-`arity` leaf of overloaded native `base_name`:
/// `"__sqrat_ol_ <base_name>_<arity>"`. The single source of truth for the key
/// format, shared by registration ([`overload_key`]) and dispatch ([`dispatch_overload`]).
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

/// The body of a Sqrat overload dispatcher: count the call's args `N` (the receiver is
/// excluded — `Variadic` only collects the real arguments), resolve
/// `"__sqrat_ol_ <base_name>_<N>"` on the *live* receiver, and call it with `this`
/// bound, forwarding the args unchanged. Re-resolving on `this` each call is what makes
/// a mod's replacement of a specific overload take effect. The args are forwarded from
/// the call's own stack slots and the leaf's result is returned in place, so no
/// references are taken.
///
/// Strict like the original engine: an arity with no matching leaf raises the engine's
/// exact error string.
pub(crate) fn dispatch_overload(
	vm: &SQVM,
	this: This,
	base_name: &str,
	args: Variadic<StackArg>,
) -> Result<Forwarded, String> {
	let found = with_overload_key(base_name, args.0.len(), |key| {
		this.forward_member(vm, key, &args.0)
	})?;
	found.ok_or_else(|| "No overload matching this argument list found".to_string())
}

/// Build the dispatcher closure for an overloaded native named `base_name`, ready to
/// slot under the bare name on a **table** (e.g. `Math.rand`). For an overloaded
/// **class method**, use `ClassBuilder::native_method` with an inline adapter instead
/// (the dispatcher must be a class member so `inst.name(...)` resolves to it).
pub(crate) fn make_dispatcher(vm: &mut SQVM, base_name: &str) -> SqStrongRef {
	let name = base_name.to_string();
	vm.make_closure(move |vm: &SQVM, this: This, args: Variadic<StackArg>| {
		dispatch_overload(vm, this, &name, args)
	})
}
