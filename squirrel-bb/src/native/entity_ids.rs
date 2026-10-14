//! Entity ids (`getID()`): one counter for the world's entities and the battle's units.
//!
//! The original takes them from a single object counter that many engine objects draw
//! from, so a settlement, a party and a brother never share an id, and none collides
//! with a faction id: settlements and factions key their rosters in the same
//! `World.getRoster(id)` space. It starts at 1, and worldgen's ~19,000 decorations (and
//! every name label) take ids before the settlements do, so a new campaign's entity ids
//! land in the tens of thousands; ours are the original's (a new campaign's ids match
//! the original's save of it).

use std::cell::Cell;
use std::rc::Rc;

/// The first id handed out, as in the original (its first decoration's).
pub const FIRST: i64 = 1;

/// The shared counter (cheap to clone; clones share it).
#[derive(Clone)]
pub struct EntityIds(Rc<Cell<i64>>);

impl Default for EntityIds {
	fn default() -> Self {
		EntityIds(Rc::new(Cell::new(FIRST)))
	}
}

impl EntityIds {
	/// Hand out the next id.
	pub(crate) fn next(&self) -> i64 {
		let id = self.0.get();
		self.0.set(id + 1);
		id
	}

	/// The id the next entity will get (what a save keeps).
	pub(crate) fn peek(&self) -> i64 {
		self.0.get()
	}

	/// Continue from `next` (a loaded save's counter).
	pub(crate) fn restore(&self, next: i64) {
		self.0.set(next);
	}
}
