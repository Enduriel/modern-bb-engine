//! The open list's binary heap, step for step as MSVC's `std::push_heap`/`pop_heap`/
//! `make_heap` do it (`_Push_heap_by_index`, `_Pop_heap_hole_by_index`), so equal-cost
//! nodes come off in the original's order and paths tie-break identically.
//!
//! `less(a, b)` is the heap's predicate; the navigator's is "a's estimate is greater",
//! making it a min-heap.

/// Sift `val` up from `hole` towards `top`.
pub(super) fn push_by_index(h: &mut [usize], mut hole: usize, top: usize, val: usize, less: &impl Fn(usize, usize) -> bool) {
	while top < hole {
		let idx = (hole - 1) / 2;
		if !less(h[idx], val) {
			break;
		}
		h[hole] = h[idx];
		hole = idx;
	}
	h[hole] = val;
}

/// Move the hole at `hole` down to the bottom (taking the larger child), then sift
/// `val` up into it. `bottom` is the heap's length.
pub(crate) fn pop_hole_by_index(h: &mut [usize], mut hole: usize, bottom: usize, val: usize, less: &impl Fn(usize, usize) -> bool) {
	let top = hole;
	let mut idx = hole;
	// (bottom - 1) >> 1, as a signed division (bottom >= 1 here).
	let max_non_leaf = bottom.saturating_sub(1) / 2;
	while idx < max_non_leaf {
		idx = 2 * idx + 2;
		if less(h[idx], h[idx - 1]) {
			idx -= 1;
		}
		h[hole] = h[idx];
		hole = idx;
	}
	if idx == max_non_leaf && bottom % 2 == 0 && bottom > 0 {
		h[hole] = h[bottom - 1];
		hole = bottom - 1;
	}
	push_by_index(h, hole, top, val, less);
}

/// `std::push_heap` after a `push_back`.
pub(super) fn push(h: &mut Vec<usize>, val: usize, less: &impl Fn(usize, usize) -> bool) {
	h.push(val);
	let n = h.len();
	if n >= 2 {
		push_by_index(h, n - 1, 0, val, less);
	}
}

/// `std::pop_heap` + `pop_back`: the top element.
pub(super) fn pop(h: &mut Vec<usize>, less: &impl Fn(usize, usize) -> bool) -> Option<usize> {
	let n = h.len();
	if n >= 2 {
		let val = h[n - 1];
		h[n - 1] = h[0];
		pop_hole_by_index(h, 0, n - 1, val, less);
	}
	h.pop()
}

/// `std::make_heap`.
pub(crate) fn make(h: &mut [usize], less: &impl Fn(usize, usize) -> bool) {
	let n = h.len();
	if n < 2 {
		return;
	}
	let mut hole = n / 2;
	while hole > 0 {
		hole -= 1;
		let val = h[hole];
		pop_hole_by_index(h, hole, n, val, less);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn pops_in_priority_order() {
		let keys = [5, 3, 8, 1, 9, 2, 7, 3, 3, 0];
		let less = |a: usize, b: usize| keys[a] > keys[b];
		let mut h = Vec::new();
		for i in 0..keys.len() {
			push(&mut h, i, &less);
		}
		let mut out = Vec::new();
		while let Some(i) = pop(&mut h, &less) {
			out.push(keys[i]);
		}
		assert_eq!(out, vec![0, 1, 2, 3, 3, 3, 5, 7, 8, 9]);
	}

	#[test]
	fn make_heap_then_pop() {
		let keys = [4, 4, 1, 6, 0, 2];
		let less = |a: usize, b: usize| keys[a] > keys[b];
		let mut h: Vec<usize> = (0..keys.len()).collect();
		make(&mut h, &less);
		assert_eq!(keys[h[0]], 0);
	}
}
