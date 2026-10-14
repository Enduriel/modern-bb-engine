//! Zones of control and of occupation, as the original keeps them per tile: a `u8`
//! count per faction and a mask of the factions exerting one. The original's are
//! 64 entries and 64 bits; here any faction id works (see [`FactionSet`]).
//!
//! `tile.addZoneOfControl(f)` doesn't mark the tile itself: it adds to each neighbour
//! within one height level. Removing decrements the count (wrapping like a `u8`) and
//! clears the faction's bit only when it reaches 0. Faction 0 (no faction) is ignored.
//! The `…OtherThan` counts sum the per-faction counts, while `has…OtherThan` reads the
//! mask (they can disagree after an unbalanced remove, as in the original).

pub(crate) use super::faction_set::FactionSet;

/// The set of a list of factions.
pub(crate) fn faction_mask(factions: &[i64]) -> FactionSet {
	FactionSet::of(factions)
}

#[derive(Clone)]
pub(crate) struct Zone {
	/// The count per faction: ids 0..64 inline, as in the original, any higher id after
	/// them (the original writes those past its array).
	pub counts: [u8; 64],
	pub high_counts: Vec<u8>,
	pub mask: FactionSet,
}

impl Default for Zone {
	fn default() -> Self {
		Zone { counts: [0; 64], high_counts: Vec::new(), mask: FactionSet::default() }
	}
}

impl Zone {
	fn count_mut(&mut self, f: i64) -> Option<&mut u8> {
		let i = usize::try_from(f).ok()?;
		if i < 64 {
			return Some(&mut self.counts[i]);
		}
		if self.high_counts.len() <= i - 64 {
			self.high_counts.resize(i - 64 + 1, 0);
		}
		Some(&mut self.high_counts[i - 64])
	}

	pub fn add(&mut self, f: i64) {
		if let Some(c) = self.count_mut(f) {
			*c = c.wrapping_add(1);
		}
		self.mask.insert(f);
	}

	pub fn remove(&mut self, f: i64) {
		if let Some(c) = self.count_mut(f) {
			*c = c.wrapping_sub(1);
			if *c == 0 {
				self.mask.remove(f);
			}
		}
	}

	pub fn count(&self, f: i64) -> i64 {
		let Ok(i) = usize::try_from(f) else { return 0 };
		let c = if i < 64 { self.counts[i] } else { self.high_counts.get(i - 64).copied().unwrap_or(0) };
		c as i64
	}

	/// Every faction's count, with its id.
	fn all_counts(&self) -> impl Iterator<Item = (i64, i64)> + '_ {
		self.counts.iter().chain(self.high_counts.iter()).enumerate().map(|(f, &c)| (f as i64, c as i64))
	}

	/// The summed counts of the factions not in `set`.
	pub fn count_outside(&self, set: &FactionSet) -> i64 {
		self.all_counts().filter(|&(f, _)| !set.contains(f)).map(|(_, c)| c).sum()
	}

	/// Whether a faction outside `set` exerts one (by the mask).
	pub fn has_outside(&self, set: &FactionSet) -> bool {
		self.mask.has_outside(set)
	}
}

#[cfg(test)]
mod tests {
	use super::{FactionSet, Zone};

	/// The original loses the bit of a faction id of 64 or more and counts it past
	/// its array (into the zone-of-occupation state); here it's a faction like any other.
	#[test]
	fn any_faction_id_has_its_own_zone() {
		let mut z = Zone::default();
		z.add(70);
		assert_eq!(z.count(70), 1);
		assert!(z.has_outside(&FactionSet::default()));
		assert!(!z.has_outside(&FactionSet::of(&[70])));
		assert_eq!(z.count_outside(&FactionSet::of(&[3])), 1);
		assert_eq!(z.count(6), 0);
		z.remove(70);
		assert_eq!(z.count(70), 0);
		assert!(!z.has_outside(&FactionSet::default()));
	}
}
