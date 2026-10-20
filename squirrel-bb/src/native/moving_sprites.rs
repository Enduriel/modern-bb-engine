//! The owners of sprites a frame may change on its own, so the frame's passes over
//! sprites (the turn: torques and animations, see `spin` and `sprite_anim`; the draw:
//! fades, see `sprite_color`) visit those and not every sprite there is.
//!
//! A sprite may change on its own while it's visible and has a torque, frames still to
//! play (or hasn't started its brush's yet) or a fade (see [`SpriteState::may_move`]):
//! a hidden sprite neither turns nor steps, as in the original, which updates only the
//! sprites it draws. Its owner (an entity, a unit, a battle tile) is noted whenever one
//! of its sprites may have started to: made, given a brush, `play`, a torque, a fade,
//! shown (`Visible`, an entity's `setVisible`), placed on the battle map, loaded. A pass visits the
//! noted owners' sprites as it visited everyone's, and the turn forgets an owner once
//! none of its sprites may change (or it's gone).
//!
//! What a visit does to one sprite doesn't depend on any other (an animation's random
//! pauses come from the sprite's own stream, see `sprite_anim`), so the order owners are
//! visited in changes nothing. Every pass runs no script, so none is noted or made
//! during one.
//!
//! [`SpriteState::may_move`]: crate::native::sprite_class::SpriteState::may_move

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::Hash;
use std::rc::Rc;

/// The owners noted, shared by a kind's sprite class (which notes them) and the passes.
pub(crate) struct Moving<K>(Rc<RefCell<Owners<K>>>);

impl<K> Clone for Moving<K> {
	fn clone(&self) -> Self {
		Moving(self.0.clone())
	}
}

impl<K> Default for Moving<K> {
	fn default() -> Self {
		Moving(Rc::new(RefCell::new(Owners {
			list: Vec::new(),
			listed: HashSet::new(),
		})))
	}
}

struct Owners<K> {
	/// In the order first noted.
	list: Vec<K>,
	listed: HashSet<K>,
}

impl<K: Copy + Eq + Hash> Moving<K> {
	/// One of `owner`'s sprites may change on its own now.
	pub(crate) fn note(&self, owner: K) {
		let mut o = self.0.borrow_mut();
		if o.listed.insert(owner) {
			o.list.push(owner);
		}
	}

	/// Visit each owner noted (a visit may not note one).
	pub(crate) fn each(&self, visit: impl FnMut(K)) {
		self.0.borrow().list.iter().copied().for_each(visit);
	}

	/// Visit each owner, keeping those `visit` says may still change.
	pub(crate) fn retain(&self, mut visit: impl FnMut(K) -> bool) {
		let mut o = self.0.borrow_mut();
		let Owners { list, listed } = &mut *o;
		list.retain(|&k| {
			let keep = visit(k);
			if !keep {
				listed.remove(&k);
			}
			keep
		});
	}

	/// Whether `owner` is noted.
	pub(crate) fn has(&self, owner: K) -> bool {
		self.0.borrow().listed.contains(&owner)
	}

	#[cfg(test)]
	pub(crate) fn len(&self) -> usize {
		self.0.borrow().list.len()
	}

	pub(crate) fn clear(&self) {
		let mut o = self.0.borrow_mut();
		o.list.clear();
		o.listed.clear();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn owners(m: &Moving<u32>) -> Vec<u32> {
		let mut v = Vec::new();
		m.each(|k| v.push(k));
		v
	}

	#[test]
	fn an_owner_is_listed_once_until_forgotten() {
		let m = Moving::<u32>::default();
		m.note(3);
		m.note(1);
		m.note(3);
		assert_eq!(owners(&m), [3, 1]);
		m.retain(|k| k != 3);
		assert_eq!(owners(&m), [1]);
		m.note(3);
		assert_eq!(owners(&m), [1, 3], "noted again once forgotten");
		m.clear();
		assert_eq!(m.len(), 0);
	}
}
