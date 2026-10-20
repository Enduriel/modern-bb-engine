//! An entity's named sprite slots, kept in the order they were added: the order its
//! sprites are layered in (e.g. a party's base, then its figure, then its banner; a
//! unit's body, then its armour, head and helmet). World and battle entities use it.

use std::rc::Rc;

use squirrel::SqStrongRef;

/// Named handles in insertion order. Entities have a handful of sprites, so lookup
/// is a linear scan. A name is shared with the scenes a renderer reads (see
/// `crate::scene`), which then copy no text.
#[derive(Default)]
pub(crate) struct Slots(Vec<(Rc<str>, SqStrongRef)>);

impl Slots {
	pub(crate) fn get(&self, name: &str) -> Option<&SqStrongRef> {
		self.0.iter().find(|(n, _)| **n == *name).map(|(_, h)| h)
	}

	pub(crate) fn contains_key(&self, name: &str) -> bool {
		self.get(name).is_some()
	}

	/// Add `name`, or replace its handle in place if it exists.
	pub(crate) fn insert(&mut self, name: String, handle: SqStrongRef) {
		match self.0.iter_mut().find(|(n, _)| **n == *name) {
			Some(slot) => slot.1 = handle,
			None => self.0.push((name.into(), handle)),
		}
	}

	/// Add `name` first, so it's drawn below the others (`insertSprite`). The caller
	/// checks it isn't there yet.
	pub(crate) fn insert_first(&mut self, name: String, handle: SqStrongRef) {
		self.0.insert(0, (name.into(), handle));
	}

	/// Remove `name`, keeping the others' order.
	pub(crate) fn remove(&mut self, name: &str) {
		self.0.retain(|(n, _)| **n != *name);
	}

	/// The slots in the order they were added.
	pub(crate) fn iter(&self) -> impl Iterator<Item = (&Rc<str>, &SqStrongRef)> {
		self.0.iter().map(|(n, h)| (n, h))
	}
}
