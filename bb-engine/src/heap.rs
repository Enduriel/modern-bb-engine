//! The C heap's usage as its allocator counts it, for the memory tests and tools: exact,
//! unlike the resident set. glibc's `mallinfo2` on Linux, the malloc zones' statistics on
//! macOS; unknown elsewhere.
//!
//! On macOS the count is exact only with the large cache off ([`exact`]): the allocator
//! keeps freed large blocks (about 1 MB and up) for reuse and hands a cached block whole
//! to a smaller request, which then counts the whole block as in use. Loading a campaign
//! frees the last world's fog mask (4.5 MB) and tile array (3.5 MB) and allocates new
//! ones, so which cached block each gets, and the count with it, flips between loads by
//! a megabyte or more. A memory test runs itself again with the cache off ([`rerun_exact`]).

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

/// The variable that turns macOS's large cache off (`0`), read once at the process's start.
#[cfg(target_os = "macos")]
const LARGE_CACHE: &str = "MallocLargeCache";

/// Whether [`bytes_in_use`] counts what the program holds, with no cached block's slack:
/// always on Linux, on macOS when the process started with the large cache off.
pub fn exact() -> bool {
	#[cfg(target_os = "macos")]
	return std::env::var_os(LARGE_CACHE).is_some_and(|v| v == "0");
	#[cfg(not(target_os = "macos"))]
	true
}

/// Runs this test binary's test `name` again in a child process where [`exact`] holds,
/// prints its output, and returns whether it passed. For a test that measures the heap and
/// finds it inexact: `if !heap::exact() { assert!(heap::rerun_exact("name")); return; }`.
pub fn rerun_exact(name: &str) -> bool {
	let exe = std::env::current_exe().expect("this test binary's path");
	let mut cmd = std::process::Command::new(exe);
	cmd.args([name, "--exact", "--nocapture", "--test-threads=1"]);
	#[cfg(target_os = "macos")]
	cmd.env(LARGE_CACHE, "0");
	let out = cmd.output().expect("run the test again");
	// Through the test's own output, so libtest shows it with the test's result.
	print!("{}", String::from_utf8_lossy(&out.stdout));
	eprint!("{}", String::from_utf8_lossy(&out.stderr));
	out.status.success()
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
