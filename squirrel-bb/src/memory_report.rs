//! What the engine's services hold, for the memory snapshot (`bb_app::memory_snapshot`):
//! each service that keeps records, caches or buffers says how many and roughly how many
//! bytes ([`ReportMemory`]). Nothing is counted until a snapshot asks.
//!
//! The bytes are the containers' own: a vector's or map's capacity times its element's
//! size, plus the text and buffers a record owns where a service counts them. Memory
//! shared between services (a brush name both the grid and the registry name) counts
//! where it is reported first. So they are estimates for finding what grows, not an
//! accounting of the heap: malloc's own figures and the VM's census sit beside them.

use std::collections::{HashMap, HashSet};

/// One thing a service holds: how many, and their bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryItem {
	/// The service (`world entities`, `sound bank`, …).
	pub owner: String,
	/// What of it (`records`, `decoded sounds`, …).
	pub what: String,
	pub count: u64,
	pub bytes: u64,
}

/// The items the services reported, in the order they did.
#[derive(Clone, Debug, Default)]
pub struct MemoryReport {
	pub items: Vec<MemoryItem>,
}

impl MemoryReport {
	/// `owner` holds `count` of `what`, taking `bytes`.
	pub fn add(&mut self, owner: &str, what: &str, count: usize, bytes: usize) {
		self.items.push(MemoryItem {
			owner: owner.to_string(),
			what: what.to_string(),
			count: count as u64,
			bytes: bytes as u64,
		});
	}

	/// The bytes reported in all.
	pub fn bytes(&self) -> u64 {
		self.items.iter().map(|i| i.bytes).sum()
	}
}

/// A service that can say what it holds.
pub trait ReportMemory {
	fn report_memory(&self, report: &mut MemoryReport);
}

/// A vector's buffer.
pub fn vec_bytes<T>(v: &Vec<T>) -> usize {
	v.capacity() * size_of::<T>()
}

/// A hash map's table: its capacity's entries and their control bytes.
pub fn map_bytes<K, V, H>(m: &HashMap<K, V, H>) -> usize {
	m.capacity() * (size_of::<(K, V)>() + 1)
}

/// A hash set's table (see [`map_bytes`]).
pub fn set_bytes<T, H>(s: &HashSet<T, H>) -> usize {
	s.capacity() * (size_of::<T>() + 1)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn items_add_up() {
		let mut report = MemoryReport::default();
		let v: Vec<u64> = Vec::with_capacity(10);
		report.add("a", "records", 3, vec_bytes(&v));
		report.add("b", "names", 1, 5);
		assert_eq!(report.items[0].bytes, 80);
		assert_eq!(report.bytes(), 85);
		let m: HashMap<u32, u32> = HashMap::with_capacity(4);
		assert!(map_bytes(&m) >= 4 * 9);
	}
}
