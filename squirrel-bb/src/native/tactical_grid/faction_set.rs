//! Sets of faction ids, for the per-faction state a battle keeps (zones of control and
//! occupation, visibility, the navigator's allies).
//!
//! The original keeps these in 64-bit masks (and 64-entry count arrays) indexed by
//! faction id without a bounds check, so a faction id of 64 or more loses its bit and
//! its count corrupts the neighbouring state (an intentional difference from the
//! original; see `docs/differences.md`). Here the first 64 ids are one word, as in the
//! original, and any higher id gets more words, allocated only when one shows up.

/// A set of non-negative faction ids. Negative ids are never members.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct FactionSet {
	/// Ids 0..64.
	low: u64,
	/// Ids 64.., 64 per word.
	high: Vec<u64>,
}

impl FactionSet {
	/// The set of the given factions.
	pub fn of(factions: &[i64]) -> Self {
		let mut set = FactionSet::default();
		for &f in factions {
			set.insert(f);
		}
		set
	}

	/// The word and bit of `f`, or `None` for a negative id.
	fn place(f: i64) -> Option<(usize, u64)> {
		let f = usize::try_from(f).ok()?;
		Some((f / 64, 1u64 << (f % 64)))
	}

	fn word(&self, i: usize) -> u64 {
		if i == 0 { self.low } else { self.high.get(i - 1).copied().unwrap_or(0) }
	}

	pub fn contains(&self, f: i64) -> bool {
		Self::place(f).is_some_and(|(i, bit)| self.word(i) & bit != 0)
	}

	pub fn insert(&mut self, f: i64) {
		let Some((i, bit)) = Self::place(f) else { return };
		if i == 0 {
			self.low |= bit;
		} else {
			if self.high.len() < i {
				self.high.resize(i, 0);
			}
			self.high[i - 1] |= bit;
		}
	}

	pub fn remove(&mut self, f: i64) {
		let Some((i, bit)) = Self::place(f) else { return };
		if i == 0 {
			self.low &= !bit;
		} else if let Some(w) = self.high.get_mut(i - 1) {
			*w &= !bit;
		}
	}

	pub fn clear(&mut self) {
		self.low = 0;
		self.high.clear();
	}

	/// Whether a member isn't in `others`.
	pub fn has_outside(&self, others: &FactionSet) -> bool {
		if self.low & !others.low != 0 {
			return true;
		}
		self.high.iter().enumerate().any(|(i, &w)| w & !others.word(i + 1) != 0)
	}

	pub fn is_empty(&self) -> bool {
		self.low == 0 && self.high.iter().all(|&w| w == 0)
	}
}

#[cfg(test)]
mod tests {
	use super::FactionSet;

	#[test]
	fn holds_any_faction_id() {
		let mut s = FactionSet::of(&[1, 63, 64, 70, 200]);
		assert!(s.contains(1) && s.contains(63) && s.contains(64) && s.contains(70) && s.contains(200));
		assert!(!s.contains(0) && !s.contains(65) && !s.contains(-1));
		s.remove(70);
		assert!(!s.contains(70));
		let mut emptied = FactionSet::of(&[70]);
		emptied.remove(70);
		assert!(emptied.is_empty() && !s.is_empty());
		assert!(s.has_outside(&FactionSet::of(&[1, 63, 64])));
		assert!(!s.has_outside(&FactionSet::of(&[1, 63, 64, 200])));
		s.clear();
		assert_eq!(s, FactionSet::default());
	}
}
