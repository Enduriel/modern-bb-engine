//! Stub-call telemetry: the natives that don't do what the original's do (a no-op, a
//! constant answer, a partial stand-in) note each call here, so a run can list the
//! stubs the scripts actually reached. Most of the battle's missing visuals were such
//! stubs (the highlighter, the shaker, `setVisibleInFogOfWar`, `buildRain`, a unit's
//! `fadeIn`, `isStoringColor`); this list is the to-do list of what the scripts ask for
//! and we ignore, visual or not.
//!
//! A stub is marked where it is made: [`closure`] for a native that does nothing,
//! [`note`] inside one that answers a constant or does only part of its job. Noting a
//! call is a lookup in a small map, cheap enough to leave on always. The record is the
//! thread's (the VM runs on one), and an [`Engine`](crate::Engine) starts it afresh.

use std::cell::RefCell;
use std::collections::BTreeMap;

use squirrel::{Retained, SQVM, SqStrongRef, Variadic};

thread_local! {
	static CALLS: RefCell<BTreeMap<&'static str, u64>> = const { RefCell::new(BTreeMap::new()) };
}

/// Note a call of the stub `name` (e.g. `"Tactical.getWeather().buildBirds"`).
pub(crate) fn note(name: &'static str) {
	CALLS.with(|c| *c.borrow_mut().entry(name).or_default() += 1);
}

/// A name made at install time, kept for the run (a stub's name built from parts).
pub(crate) fn name(name: String) -> &'static str {
	Box::leak(name.into_boxed_str())
}

/// A native that does nothing with whatever it is given, noting each call as `name`.
pub(crate) fn closure(vm: &mut SQVM, name: &'static str) -> SqStrongRef {
	vm.make_closure(move |_args: Variadic<Retained>| note(name))
}

/// The stubs called since the record started, with their call counts, by name.
pub fn calls() -> Vec<(String, u64)> {
	CALLS.with(|c| c.borrow().iter().map(|(k, v)| ((*k).to_string(), *v)).collect())
}

/// Start the record afresh.
pub(crate) fn reset() {
	CALLS.with(|c| c.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn calls_are_counted_by_name() {
		reset();
		note("World.a");
		note("World.b");
		note("World.a");
		assert_eq!(calls(), [("World.a".to_string(), 2), ("World.b".to_string(), 1)]);
		reset();
		assert!(calls().is_empty());
	}
}
