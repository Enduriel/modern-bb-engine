//! Zones of control and of occupation, as the original keeps them per tile: a `u8`
//! count per faction (0..64) and a 64-bit mask of the factions exerting one.
//!
//! `tile.addZoneOfControl(f)` doesn't mark the tile itself: it adds to each neighbour
//! within one height level. Removing decrements the count (wrapping like a `u8`) and
//! clears the faction's bit only when it reaches 0. Faction 0 (no faction) is ignored.
//! The `…OtherThan` counts sum the per-faction counts, while `has…OtherThan` reads the
//! mask (they can disagree after an unbalanced remove, as in the original).

/// The bit for faction `f` (none for factions outside 0..64, like the x86 shifts).
pub(crate) fn faction_bit(f: i64) -> u64 {
	if (0..64).contains(&f) { 1u64 << f } else { 0 }
}

/// The mask of a list of factions.
pub(crate) fn faction_mask(factions: &[i64]) -> u64 {
	factions.iter().fold(0, |m, &f| m | faction_bit(f))
}

#[derive(Clone)]
pub(crate) struct Zone {
	pub counts: [u8; 64],
	pub mask: u64,
}

impl Default for Zone {
	fn default() -> Self {
		Zone { counts: [0; 64], mask: 0 }
	}
}

impl Zone {
	pub fn add(&mut self, f: i64) {
		if let Some(c) = usize::try_from(f).ok().and_then(|i| self.counts.get_mut(i)) {
			*c = c.wrapping_add(1);
		}
		self.mask |= faction_bit(f);
	}

	pub fn remove(&mut self, f: i64) {
		if let Some(c) = usize::try_from(f).ok().and_then(|i| self.counts.get_mut(i)) {
			*c = c.wrapping_sub(1);
			if *c == 0 {
				self.mask &= !faction_bit(f);
			}
		}
	}

	pub fn count(&self, f: i64) -> i64 {
		usize::try_from(f).ok().and_then(|i| self.counts.get(i)).map_or(0, |&c| c as i64)
	}

	/// The summed counts of the factions in `mask`.
	pub fn count_in(&self, mask: u64) -> i64 {
		(0..64).filter(|&f| mask & (1u64 << f) != 0).map(|f| self.counts[f] as i64).sum()
	}

	/// The summed counts of the factions not in `mask`.
	pub fn count_outside(&self, mask: u64) -> i64 {
		self.count_in(!mask)
	}

	/// Whether a faction outside `mask` exerts one (by the mask).
	pub fn has_outside(&self, mask: u64) -> bool {
		self.mask & !mask != 0
	}
}
