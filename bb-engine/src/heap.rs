//! The C heap's usage as its allocator counts it, for the memory tests and tools: exact,
//! unlike the resident set. glibc's `mallinfo2` on Linux, the malloc zones' statistics on
//! macOS; unknown elsewhere.

/// The C heap's bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeapUsage {
	/// Handed out and not given back (large blocks mapped on their own included).
	pub in_use: usize,
	/// Held by the allocator but free.
	pub free: usize,
}

/// The C heap's usage now, if this system's allocator reports it.
pub fn usage() -> Option<HeapUsage> {
	imp::usage()
}

/// Bytes in use (see [`usage`]), or 0 where unknown.
pub fn bytes_in_use() -> usize {
	usage().map_or(0, |u| u.in_use)
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod imp {
	use super::HeapUsage;

	#[repr(C)]
	struct Mallinfo2 {
		arena: usize,
		ordblks: usize,
		smblks: usize,
		hblks: usize,
		hblkhd: usize,
		usmblks: usize,
		fsmblks: usize,
		uordblks: usize,
		fordblks: usize,
		keepcost: usize,
	}

	unsafe extern "C" {
		fn mallinfo2() -> Mallinfo2;
	}

	pub(super) fn usage() -> Option<HeapUsage> {
		// SAFETY: `mallinfo2` only reads the allocator's counters.
		let m = unsafe { mallinfo2() };
		Some(HeapUsage { in_use: m.uordblks + m.hblkhd, free: m.fordblks })
	}
}

#[cfg(target_os = "macos")]
mod imp {
	use std::ffi::{c_uint, c_void};

	use super::HeapUsage;

	/// `malloc_statistics_t` (`<malloc/malloc.h>`).
	#[repr(C)]
	#[derive(Default)]
	struct MallocStatistics {
		blocks_in_use: c_uint,
		size_in_use: usize,
		max_size_in_use: usize,
		size_allocated: usize,
	}

	unsafe extern "C" {
		fn malloc_zone_statistics(zone: *mut c_void, stats: *mut MallocStatistics);
	}

	pub(super) fn usage() -> Option<HeapUsage> {
		let mut s = MallocStatistics::default();
		// SAFETY: a null zone sums every zone's statistics into `s`.
		unsafe { malloc_zone_statistics(std::ptr::null_mut(), &mut s) };
		Some(HeapUsage { in_use: s.size_in_use, free: s.size_allocated.saturating_sub(s.size_in_use) })
	}
}

#[cfg(not(any(all(target_os = "linux", target_env = "gnu"), target_os = "macos")))]
mod imp {
	pub(super) fn usage() -> Option<super::HeapUsage> {
		None
	}
}
