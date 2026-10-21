//! The heap count ([`heap`]) is exact where the memory tests need it: a block counts its
//! own size, not a bigger freed block's that the allocator reused for it (macOS's large
//! cache, see [`heap::exact`]). One test in its own binary, so no other test allocates
//! meanwhile.

use std::alloc::{GlobalAlloc, Layout, System};

use bb_engine::heap;

#[test]
fn a_large_block_counts_its_own_size() {
	if !heap::exact() {
		assert!(heap::rerun_exact("a_large_block_counts_its_own_size"), "the test failed with an exact heap count");
		return;
	}
	if heap::usage().is_none() {
		return;
	}
	// The sizes of a campaign's fog mask and tile array: with the cache on, the smaller one
	// gets the bigger one's freed block, and counts its whole size.
	let (big, small) = (Layout::from_size_align(4_734_976, 16).unwrap(), Layout::from_size_align(3_620_864, 16).unwrap());
	let before = heap::bytes_in_use();
	// SAFETY: plain allocations of non-zero sizes, each freed once with its own layout.
	// Through `black_box`, so the compiler keeps them.
	let grew = unsafe {
		let b = std::hint::black_box(System.alloc(big));
		assert!(!b.is_null());
		System.dealloc(b, big);
		let s = std::hint::black_box(System.alloc(small));
		assert!(!s.is_null());
		let grew = heap::bytes_in_use().saturating_sub(before);
		System.dealloc(s, small);
		grew
	};
	assert!(grew < small.size() + 64 * 1024, "a block of {} bytes counts as {grew} in use", small.size());
}
