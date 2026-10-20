//! Injecting engine-native methods into BB class instances, the way the original
//! engine does it (probed in the real game, 2026-10-13): the natives live as raw slots
//! on the instance's **base** class-level table (e.g. `entity`), below every script
//! level, so script methods of the same name always win.
//!
//! The shared native closures are bound to the instance's leaf, so whichever level a
//! call is looked up on, the native sees the leaf as `this` and
//! [`SQVM::this_identity`](squirrel::SQVM::this_identity) resolves one record.
//!
//! **Bound when first read.** Where the base level is a lazy level (see
//! `class.rs`), the natives are given to it with [`SqTable::lazy_inject`]: it behaves as
//! if each had been bound and added, but a native is bound only when the instance first
//! reads it (most instances read few of them), and the unbound set is shared by every
//! instance given the same names.

use std::cell::RefCell;
use std::collections::HashMap;

use squirrel::{CensusTrack, SQVM, SqArray, SqStrongRef, SqTable};

use crate::class::instance_levels;

/// A host's natives for injection, and the sets of them it gave lazily: for each set of
/// names (as indices into `methods`), a table holding exactly those natives, unbound, and
/// their names in order, shared by every instance given that set. Holds VM references:
/// drop it (or replace it) before the VM closes.
#[derive(Default)]
pub(crate) struct Natives {
	methods: Vec<(&'static str, SqStrongRef)>,
	sets: RefCell<HashMap<Vec<u16>, (SqStrongRef, SqStrongRef)>>,
}

impl Natives {
	/// A name listed twice is given once, the first time (as setting them one by one,
	/// skipping names the instance has, would).
	pub(crate) fn new(mut methods: Vec<(&'static str, SqStrongRef)>) -> Self {
		let mut seen = std::collections::HashSet::new();
		methods.retain(|(name, _)| seen.insert(*name));
		assert!(methods.len() <= u16::MAX as usize, "too many natives to inject");
		Natives { methods, sets: RefCell::default() }
	}

	/// The shared slots table and key array for the natives at `indices`, made once.
	fn set(&self, vm: &SQVM, indices: &[u16]) -> (SqStrongRef, SqStrongRef) {
		if let Some(set) = self.sets.borrow().get(indices) {
			return set.clone();
		}
		let mut slots = SqTable::new(vm);
		let mut keys = SqArray::new(vm);
		for &i in indices {
			let (name, closure) = &self.methods[i as usize];
			slots.set(name, closure.clone());
			keys.push(name.to_string());
		}
		let set = (slots.to_strong(), keys.to_strong());
		self.sets.borrow_mut().insert(indices.to_vec(), set.clone());
		set
	}
}

/// Add the natives at `indices` of `natives` to the instance level `base`, each bound to
/// `leaf`: lazily where `base` is a lazy level, else bound and set now. In both cases the
/// level ends up as if each had been bound and added in order.
pub(crate) fn add_bound(vm: &SQVM, base: &mut SqTable, leaf: &SqTable, natives: &Natives, indices: &[u16]) {
	if indices.is_empty() {
		return;
	}
	let (slots, keys) = natives.set(vm, indices);
	let given = match (slots.as_table(vm), keys.as_array(vm)) {
		(Some(slots), Some(keys)) => base.lazy_inject(&slots, &keys, leaf),
		_ => false,
	};
	if given {
		return;
	}
	for &i in indices {
		let (name, closure) = &natives.methods[i as usize];
		if let Some(bound) = closure.bindenv(vm, leaf) {
			base.set(name, bound);
		}
	}
}

/// Inject `natives` into the instance `leaf`. A name any class level already defines
/// is skipped: many names that look like engine primitives are script methods that
/// call the real natives (e.g. actor's `addDefaultStatusSprites` → native
/// `addSprite`), and overwriting them would skip real setup.
///
/// Returns the identities of all the instance's level tables (leaf first), for hosts
/// that also key records by level.
pub(crate) fn inject_natives(vm: &SQVM, leaf: &SqTable, natives: &Natives) -> Vec<i64> {
	vm.census_track(CensusTrack::Pause);
	let leaf_ref = leaf.to_strong();
	let levels = instance_levels(leaf_ref.as_table(vm).expect("leaf is a table"));
	let identities = levels.iter().map(|t| t.to_strong().identity()).collect();
	let mut base = levels.into_iter().last().expect("an instance has at least one level");
	let indices: Vec<u16> = (0..natives.methods.len() as u16)
		.filter(|&i| leaf.get::<SqStrongRef>(natives.methods[i as usize].0).is_none())
		.collect();
	add_bound(vm, &mut base, leaf, natives, &indices);
	vm.census_track(CensusTrack::Resume);
	identities
}
