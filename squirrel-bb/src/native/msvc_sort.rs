//! `std::sort` step for step as the original's MSVC runtime does it (the VS2015/2017
//! introsort: `_Sort_unchecked1`, `_Partition_by_median_guess_unchecked`,
//! `_Insertion_sort_unchecked`, heap sort past `1.5 log2 n` divisions), so elements the
//! predicate can't tell apart end up in the original's order.
//!
//! Disassembled from the save listing's sort (`0x568fa0`): ranges of at most 32 are
//! insertion-sorted (stable); longer ones are partitioned around a median of three, or
//! of nine past 40 elements (`0x569990`), which can reorder equal elements; `ideal`
//! starts at the length and goes `ideal / 2 + ideal / 4` per division.
//!
//! It sorts indices: `less(a, b)` compares the items at indices `a` and `b`.

use super::tactical_nav::heap;

/// Below this length a range is insertion-sorted (`_ISORT_MAX`).
const ISORT_MAX: usize = 32;

/// Sort `v` with strict weak order `less`, as MSVC's `std::sort`.
pub(crate) fn sort(v: &mut [usize], less: &impl Fn(usize, usize) -> bool) {
	let n = v.len();
	sort_range(v, 0, n, n, less);
}

/// `_Sort_unchecked1`: divide by quicksort while `ideal` lasts, recursing on the shorter
/// side and looping on the longer.
fn sort_range(v: &mut [usize], mut first: usize, mut last: usize, mut ideal: usize, less: &impl Fn(usize, usize) -> bool) {
	while last - first > ISORT_MAX && ideal > 0 {
		let (mid_first, mid_last) = partition(v, first, last, less);
		ideal /= 2;
		ideal += ideal / 2;
		if mid_first - first < last - mid_last {
			sort_range(v, first, mid_first, ideal, less);
			first = mid_last;
		} else {
			sort_range(v, mid_last, last, ideal, less);
			last = mid_first;
		}
	}
	if last - first > ISORT_MAX {
		let h = &mut v[first..last];
		heap::make(h, less);
		for end in (2..=h.len()).rev() {
			let val = h[end - 1];
			h[end - 1] = h[0];
			heap::pop_hole_by_index(h, 0, end - 1, val, less);
		}
	} else if last - first >= 2 {
		insertion_sort(v, first, last, less);
	}
}

/// `_Insertion_sort_unchecked`.
fn insertion_sort(v: &mut [usize], first: usize, last: usize, less: &impl Fn(usize, usize) -> bool) {
	for next in first + 1..last {
		let val = v[next];
		if less(val, v[first]) {
			v.copy_within(first..next, first + 1);
			v[first] = val;
		} else {
			let mut hole = next;
			while less(val, v[hole - 1]) {
				v[hole] = v[hole - 1];
				hole -= 1;
			}
			v[hole] = val;
		}
	}
}

/// `_Med3_unchecked`.
fn med3(v: &mut [usize], a: usize, b: usize, c: usize, less: &impl Fn(usize, usize) -> bool) {
	if less(v[b], v[a]) {
		v.swap(b, a);
	}
	if less(v[c], v[b]) {
		v.swap(c, b);
		if less(v[b], v[a]) {
			v.swap(b, a);
		}
	}
}

/// `_Guess_median_unchecked` over `first`, `mid` and `last` (the last element, inclusive).
fn guess_median(v: &mut [usize], first: usize, mid: usize, last: usize, less: &impl Fn(usize, usize) -> bool) {
	let count = last - first;
	if count > 40 {
		let step = (count + 1) >> 3;
		let two = step << 1;
		med3(v, first, first + step, first + two, less);
		med3(v, mid - step, mid, mid + step, less);
		med3(v, last - two, last - step, last, less);
		med3(v, first + step, mid, last - step, less);
	} else {
		med3(v, first, mid, last, less);
	}
}

/// `_Partition_by_median_guess_unchecked`: the range of elements equal to the pivot,
/// with the smaller before it and the greater after.
fn partition(v: &mut [usize], first: usize, last: usize, less: &impl Fn(usize, usize) -> bool) -> (usize, usize) {
	let mid = first + ((last - first) >> 1);
	guess_median(v, first, mid, last - 1, less);
	let mut pfirst = mid;
	let mut plast = pfirst + 1;
	while first < pfirst && !less(v[pfirst - 1], v[pfirst]) && !less(v[pfirst], v[pfirst - 1]) {
		pfirst -= 1;
	}
	while plast < last && !less(v[plast], v[pfirst]) && !less(v[pfirst], v[plast]) {
		plast += 1;
	}
	let mut gfirst = plast;
	let mut glast = pfirst;
	loop {
		while gfirst < last {
			if less(v[pfirst], v[gfirst]) {
			} else if less(v[gfirst], v[pfirst]) {
				break;
			} else {
				if plast != gfirst {
					v.swap(plast, gfirst);
				}
				plast += 1;
			}
			gfirst += 1;
		}
		while first < glast {
			if less(v[glast - 1], v[pfirst]) {
			} else if less(v[pfirst], v[glast - 1]) {
				break;
			} else {
				pfirst -= 1;
				if pfirst != glast - 1 {
					v.swap(pfirst, glast - 1);
				}
			}
			glast -= 1;
		}
		if glast == first && gfirst == last {
			return (pfirst, plast);
		}
		if glast == first {
			// No room at the bottom: rotate the pivot up.
			if plast != gfirst {
				v.swap(pfirst, plast);
			}
			plast += 1;
			v.swap(pfirst, gfirst);
			pfirst += 1;
			gfirst += 1;
		} else if gfirst == last {
			// No room at the top: rotate the pivot down.
			glast -= 1;
			pfirst -= 1;
			if glast != pfirst {
				v.swap(glast, pfirst);
			}
			plast -= 1;
			v.swap(pfirst, plast);
		} else {
			glast -= 1;
			v.swap(gfirst, glast);
			gfirst += 1;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A small xorshift, for reproducible test data.
	fn keys(n: usize, modulo: u64, mut s: u64) -> Vec<u64> {
		(0..n)
			.map(|_| {
				s ^= s << 13;
				s ^= s >> 7;
				s ^= s << 17;
				s % modulo
			})
			.collect()
	}

	#[test]
	fn sorts_like_any_sort_by_key() {
		for (n, modulo, seed) in [(0, 1, 1), (1, 1, 2), (5, 3, 3), (33, 4, 4), (41, 1000, 5), (200, 7, 6), (1000, 1_000_000, 7), (1000, 1, 8)] {
			let k = keys(n, modulo, seed);
			let mut v: Vec<usize> = (0..n).collect();
			sort(&mut v, &|a, b| k[a] > k[b]);
			let got: Vec<u64> = v.iter().map(|&i| k[i]).collect();
			let mut want = k.clone();
			want.sort_by(|a, b| b.cmp(a));
			assert_eq!(got, want, "n {n}");
			let mut seen = v.clone();
			seen.sort();
			assert_eq!(seen, (0..n).collect::<Vec<_>>());
		}
	}

	#[test]
	fn short_ranges_keep_equal_elements_in_order() {
		let k = [3, 1, 3, 2, 1, 3, 2, 2, 1, 3, 1, 1, 2, 3, 3, 1, 2, 1, 3, 2, 2, 3, 1, 1, 3, 2, 1, 3, 3, 2, 1, 1];
		let mut v: Vec<usize> = (0..k.len()).collect();
		sort(&mut v, &|a, b| k[a] < k[b]);
		let mut want: Vec<usize> = (0..k.len()).collect();
		want.sort_by_key(|&i| k[i]);
		assert_eq!(v, want);
	}

	/// Probed 2026-10-20: the original's `queryStorages` over a folder of 132 saves (a
	/// player's, so the names are left out). `TIMES` are their modification times in
	/// listing order (by file name); `ORDER` is the order the original returned them
	/// in. Two saves share a time (indices 14 and 15), and the original returns 15
	/// first, where a stable sort keeps 14 first.
	#[test]
	fn matches_the_originals_save_list() {
		const TIMES: [u64; 132] = [
			1672179067, 1613176595, 1613240031, 1622392568, 1656324831, 1717833909, 1605367517, 1658911382, 1658493986, 1600023151, 1780898298,
			1780858426, 1780898604, 1717956703, 1717399928, 1717399928, 1717524773, 1719212562, 1719212585, 1719212664, 1719212721, 1719212770,
			1613370545, 1656787623, 1717595643, 1598969991, 1612924980, 1613148389, 1621942827, 1622041197, 1622418612, 1624196103, 1623939341,
			1613357626, 1682974940, 1620446307, 1729905846, 1718030835, 1780223035, 1780228912, 1780322480, 1707655627, 1709741500, 1717452029,
			1791926330, 1780903042, 1780902325, 1780902645, 1720079747, 1717526919, 1693507373, 1623643359, 1687948625, 1720079701, 1792481788,
			1791949918, 1716979935, 1727210645, 1722606059, 1722719669, 1780254175, 1780254072, 1718014020, 1717588221, 1719217621, 1718864583,
			1603703553, 1621810248, 1599435856, 1598285170, 1598917485, 1658949332, 1780223630, 1780902901, 1780901382, 1780902287, 1718986122,
			1622066344, 1717020997, 1659754887, 1780228955, 1673175472, 1624198307, 1719250053, 1684537429, 1780858517, 1780859378, 1780867422,
			1780903311, 1621808245, 1613356164, 1717524738, 1717363180, 1720379526, 1717401872, 1717761548, 1685115779, 1717401476, 1718264650,
			1780219136, 1725448037, 1780314202, 1780315806, 1780319521, 1780321986, 1780328657, 1780331412, 1780331691, 1780332007, 1780343308,
			1780381516, 1780383460, 1780388879, 1780389438, 1780396618, 1780409334, 1780410117, 1780411689, 1780412514, 1780569206, 1780573802,
			1780224020, 1780574631, 1780576979, 1780838294, 1780249827, 1780225151, 1780291128, 1780299603, 1780306369, 1780307135, 1780223384,
		];
		const ORDER: [usize; 132] = [
			54, 55, 44, 88, 45, 73, 47, 46, 75, 74, 12, 10, 87, 86, 85, 11, 124, 123, 122, 120, 119, 118, 117, 116, 115, 114, 113, 112, 111, 110,
			109, 108, 107, 106, 105, 40, 104, 103, 102, 101, 130, 129, 128, 127, 60, 61, 125, 80, 39, 126, 121, 72, 131, 38, 99, 36, 57, 100, 59,
			58, 93, 48, 53, 83, 64, 21, 20, 19, 18, 17, 76, 65, 98, 37, 62, 13, 5, 95, 24, 63, 49, 16, 91, 43, 94, 97, 15, 14, 92, 78, 56, 42,
			41, 50, 52, 96, 84, 34, 81, 0, 79, 71, 7, 8, 23, 4, 82, 31, 32, 51, 30, 3, 77, 29, 28, 67, 89, 35, 22, 33, 90, 2, 1, 27, 26, 6, 66,
			9, 68, 25, 70, 69,
		];
		let mut v: Vec<usize> = (0..TIMES.len()).collect();
		sort(&mut v, &|a, b| TIMES[a] > TIMES[b]);
		assert_eq!(v, ORDER);
	}

	#[test]
	fn long_ranges_can_reorder_equal_elements() {
		// 33 elements: one partition, so it isn't Rust's stable order. (The order
		// itself is MSVC's, checked through the save list against the original.)
		let k: Vec<u32> = (0..33).map(|i| u32::from(i % 2 == 0)).collect();
		let mut v: Vec<usize> = (0..k.len()).collect();
		sort(&mut v, &|a, b| k[a] < k[b]);
		let mut stable: Vec<usize> = (0..k.len()).collect();
		stable.sort_by_key(|&i| k[i]);
		assert_ne!(v, stable);
		assert!(v.windows(2).all(|w| k[w[0]] <= k[w[1]]));
	}
}
