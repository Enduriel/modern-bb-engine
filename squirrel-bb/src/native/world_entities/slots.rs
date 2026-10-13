//! An entity's named sprite slots, kept in the order they were added: the order its
//! sprites are layered in (e.g. a party's base, then its figure, then its banner).

use squirrel::SqStrongRef;

/// Named handles in insertion order. Entities have a handful of sprites, so lookup
/// is a linear scan.
#[derive(Default)]
pub(super) struct Slots(Vec<(String, SqStrongRef)>);

impl Slots {
	pub(super) fn get(&self, name: &str) -> Option<&SqStrongRef> {
		self.0.iter().find(|(n, _)| n == name).map(|(_, h)| h)
	}

	pub(super) fn contains_key(&self, name: &str) -> bool {
		self.get(name).is_some()
	}

	/// Add `name`, or replace its handle in place if it exists.
	pub(super) fn insert(&mut self, name: String, handle: SqStrongRef) {
		match self.0.iter_mut().find(|(n, _)| *n == name) {
			Some(slot) => slot.1 = handle,
			None => self.0.push((name, handle)),
		}
	}

	/// The slots in the order they were added.
	pub(super) fn iter(&self) -> impl Iterator<Item = (&String, &SqStrongRef)> {
		self.0.iter().map(|(n, h)| (n, h))
	}
}
