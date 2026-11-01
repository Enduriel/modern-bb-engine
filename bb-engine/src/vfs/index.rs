//! An archive's list of files, compact: every name in one string, a fixed record per entry
//! and a sorted lookup, about 90 bytes an entry where the `zip` crate keeps ~440 (vanilla's
//! archives: ~1 MB instead of ~4.7).
//!
//! Entries keep the central directory's order (the game lists files in it). A name the
//! directory holds twice keeps its first place with the later entry's data, as the `zip`
//! crate's map has it.

use std::collections::HashMap;

/// One file in the archive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Entry {
	/// Where its name is in [`Index::names`]: start and length.
	pub(super) name: (u32, u32),
	/// The general purpose flags (bit 0 encrypted, bit 3 sizes in a data descriptor).
	pub(super) flags: u16,
	/// 0 stored, 8 deflated.
	pub(super) method: u16,
	pub(super) crc32: u32,
	pub(super) compressed: u64,
	pub(super) size: u64,
	/// Its local header's offset in the file.
	pub(super) header: u64,
	/// Its last-modified date and time as the directory records them, in MS-DOS form
	/// (date in the high 16 bits, time in the low).
	pub(super) modified: u32,
}

impl Entry {
	pub(super) fn encrypted(&self) -> bool {
		self.flags & 1 != 0
	}

	/// Whether its sizes are only in a data descriptor after its data (the directory
	/// still has them).
	pub(super) fn sized_after(&self) -> bool {
		self.flags & (1 << 3) != 0
	}
}

pub(super) struct Index {
	names: String,
	entries: Vec<Entry>,
	/// The entries' positions, sorted by name.
	sorted: Vec<u32>,
}

impl Index {
	pub(super) fn builder() -> Builder {
		Builder::default()
	}

	fn name(&self, entry: &Entry) -> &str {
		&self.names[entry.name.0 as usize..(entry.name.0 + entry.name.1) as usize]
	}

	/// The entry at `name`.
	pub(super) fn get(&self, name: &str) -> Option<&Entry> {
		let at = self.sorted.binary_search_by(|&i| self.name(&self.entries[i as usize]).cmp(name)).ok()?;
		Some(&self.entries[self.sorted[at] as usize])
	}

	pub(super) fn len(&self) -> usize {
		self.entries.len()
	}

	/// Every name, in the directory's order.
	pub(super) fn names(&self) -> impl Iterator<Item = &str> {
		self.entries.iter().map(|e| self.name(e))
	}
}

#[derive(Default)]
pub(super) struct Builder {
	names: String,
	entries: Vec<Entry>,
	seen: HashMap<String, usize>,
}

impl Builder {
	pub(super) fn add(&mut self, name: &str, mut entry: Entry) {
		if let Some(&at) = self.seen.get(name) {
			entry.name = self.entries[at].name;
			self.entries[at] = entry;
			return;
		}
		entry.name = (self.names.len() as u32, name.len() as u32);
		self.names.push_str(name);
		self.seen.insert(name.to_string(), self.entries.len());
		self.entries.push(entry);
	}

	pub(super) fn build(self) -> Index {
		let mut index = Index {
			names: self.names,
			entries: self.entries,
			sorted: Vec::new(),
		};
		let mut sorted: Vec<u32> = (0..index.entries.len() as u32).collect();
		sorted.sort_by(|&a, &b| index.name(&index.entries[a as usize]).cmp(index.name(&index.entries[b as usize])));
		index.names.shrink_to_fit();
		index.entries.shrink_to_fit();
		index.sorted = sorted;
		index
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn entry(header: u64) -> Entry {
		Entry {
			name: (0, 0),
			flags: 0,
			method: 0,
			crc32: 0,
			compressed: 1,
			size: 1,
			header,
			modified: 0,
		}
	}

	#[test]
	fn keeps_the_directorys_order_and_a_repeated_names_first_place() {
		let mut b = Index::builder();
		b.add("b.txt", entry(10));
		b.add("a.txt", entry(20));
		b.add("b.txt", entry(30));
		let index = b.build();
		assert_eq!(index.names().collect::<Vec<_>>(), ["b.txt", "a.txt"]);
		assert_eq!(index.get("b.txt").unwrap().header, 30, "the later entry's data");
		assert_eq!(index.get("a.txt").unwrap().header, 20);
		assert!(index.get("c.txt").is_none());
		assert_eq!(index.len(), 2);
	}
}
