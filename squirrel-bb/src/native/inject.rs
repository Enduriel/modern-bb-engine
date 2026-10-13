//! Injecting engine-native methods into BB class instances, the way the original
//! engine does it (probed in the real game, 2026-10-13): the natives live as raw slots
//! on the instance's **base** class-level table (e.g. `entity`), below every script
//! level, so script methods of the same name always win.
//!
//! The shared native closures are bound to the instance's leaf, so whichever level a
//! call is looked up on, the native sees the leaf as `this` and
//! [`SQVM::this_identity`](squirrel::SQVM::this_identity) resolves one record.

use squirrel::{SQVM, SqStrongRef, SqTable};

use crate::class::instance_levels;

/// Inject `methods` into the instance `leaf`. A name any class level already defines
/// is skipped: many names that look like engine primitives are script methods that
/// call the real natives (e.g. actor's `addDefaultStatusSprites` → native
/// `addSprite`), and overwriting them would skip real setup.
///
/// Returns the identities of all the instance's level tables (leaf first), for hosts
/// that also key records by level.
pub(crate) fn inject_natives(vm: &SQVM, leaf: &SqTable, methods: &[(&str, SqStrongRef)]) -> Vec<i64> {
	let leaf_ref = leaf.to_strong();
	let levels = instance_levels(leaf_ref.as_table(vm).expect("leaf is a table"));
	let identities = levels.iter().map(|t| t.to_strong().identity()).collect();
	let mut base = levels.into_iter().last().expect("an instance has at least one level");
	for (name, closure) in methods {
		if leaf.get::<SqStrongRef>(name).is_some() {
			continue;
		}
		if let Some(bound) = closure.bindenv(vm, leaf) {
			base.set(name, bound);
		}
	}
	identities
}
