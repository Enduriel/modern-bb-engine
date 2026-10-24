//! A page's free space, in [`UNIT`](super::alloc::UNIT)s: its free rectangles, each as
//! large as the regions around it allow (the "maximal rectangles" of rectangle packing),
//! and which units are taken.
//!
//! A region goes where it leaves the least over on its shorter side (best short side
//! fit): it fills a gap it nearly fits before a large open space. Every free rectangle it
//! overlaps is cut into the up to four around it. A region given back is free again; the
//! free rectangle put back for it reaches as far as the free units around it allow, so a
//! larger region fits the hole it and its free neighbours leave.

/// A rectangle, in units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
	pub x: u32,
	pub y: u32,
	pub w: u32,
	pub h: u32,
}

impl Rect {
	fn right(&self) -> u32 {
		self.x + self.w
	}

	fn bottom(&self) -> u32 {
		self.y + self.h
	}

	fn contains(&self, o: &Rect) -> bool {
		o.x >= self.x && o.y >= self.y && o.right() <= self.right() && o.bottom() <= self.bottom()
	}

	fn overlaps(&self, o: &Rect) -> bool {
		o.x < self.right() && self.x < o.right() && o.y < self.bottom() && self.y < o.bottom()
	}
}

/// How well a `w` × `h` region fits a free rectangle: what it leaves over on its shorter
/// side, then on its longer; less is better.
pub(crate) type Fit = (u32, u32);

/// Which units are taken: a bit each, a row's in whole words.
#[derive(Clone, Debug)]
struct Taken {
	words: usize,
	bits: Vec<u64>,
}

impl Taken {
	fn new(w: u32, h: u32) -> Taken {
		let words = w.div_ceil(64) as usize;
		Taken {
			words,
			bits: vec![0; words * h as usize],
		}
	}

	/// Whether units `x0..x1` of row `y` are all free.
	fn row_free(&self, y: u32, x0: u32, x1: u32) -> bool {
		let row = &self.bits[y as usize * self.words..][..self.words];
		(x0..x1).all(|x| row[(x / 64) as usize] & (1 << (x % 64)) == 0)
	}

	fn column_free(&self, x: u32, y0: u32, y1: u32) -> bool {
		(y0..y1).all(|y| self.row_free(y, x, x + 1))
	}

	fn set(&mut self, r: &Rect, taken: bool) {
		for y in r.y..r.bottom() {
			let row = &mut self.bits[y as usize * self.words..][..self.words];
			for x in r.x..r.right() {
				let (word, bit) = ((x / 64) as usize, 1u64 << (x % 64));
				if taken {
					row[word] |= bit;
				} else {
					row[word] &= !bit;
				}
			}
		}
	}
}

/// A page's free space.
#[derive(Clone, Debug)]
pub(crate) struct FreeSpace {
	w: u32,
	h: u32,
	free: Vec<Rect>,
	taken: Taken,
}

impl FreeSpace {
	/// An empty page `w` × `h` units.
	pub(crate) fn new(w: u32, h: u32) -> FreeSpace {
		FreeSpace {
			w,
			h,
			free: vec![Rect { x: 0, y: 0, w, h }],
			taken: Taken::new(w, h),
		}
	}

	/// The best place for a `w` × `h` region, and how well it fits; `None` if it fits
	/// nowhere. Of places that fit as well, the highest, then the leftmost.
	pub(crate) fn best(&self, w: u32, h: u32) -> Option<(Fit, u32, u32)> {
		self.free
			.iter()
			.filter(|f| f.w >= w && f.h >= h)
			.map(|f| {
				let (dw, dh) = (f.w - w, f.h - h);
				((dw.min(dh), dw.max(dh)), f.y, f.x)
			})
			.min()
			.map(|(fit, y, x)| (fit, x, y))
	}

	/// Take `r`, which must be free.
	pub(crate) fn take(&mut self, r: Rect) {
		self.taken.set(&r, true);
		let old = self.free.len();
		let mut i = 0;
		let mut kept = 0;
		// Each free rectangle `r` overlaps becomes the parts of it around `r`.
		let mut cut = Vec::new();
		while i < old {
			let f = self.free[i];
			if f.overlaps(&r) {
				if r.x > f.x {
					cut.push(Rect { w: r.x - f.x, ..f });
				}
				if r.right() < f.right() {
					cut.push(Rect {
						x: r.right(),
						w: f.right() - r.right(),
						..f
					});
				}
				if r.y > f.y {
					cut.push(Rect { h: r.y - f.y, ..f });
				}
				if r.bottom() < f.bottom() {
					cut.push(Rect {
						y: r.bottom(),
						h: f.bottom() - r.bottom(),
						..f
					});
				}
			} else {
				self.free[kept] = f;
				kept += 1;
			}
			i += 1;
		}
		self.free.truncate(kept);
		self.add(cut);
	}

	/// Give `r` back.
	pub(crate) fn give_back(&mut self, r: Rect) {
		self.taken.set(&r, false);
		// As wide as the free units allow, then as tall; and the other way round.
		let across = self.grow_down(self.grow_across(r));
		let down = self.grow_across(self.grow_down(r));
		self.add(vec![across, down]);
	}

	fn grow_across(&self, mut r: Rect) -> Rect {
		while r.x > 0 && self.taken.column_free(r.x - 1, r.y, r.bottom()) {
			r.x -= 1;
			r.w += 1;
		}
		while r.right() < self.w && self.taken.column_free(r.right(), r.y, r.bottom()) {
			r.w += 1;
		}
		r
	}

	fn grow_down(&self, mut r: Rect) -> Rect {
		while r.y > 0 && self.taken.row_free(r.y - 1, r.x, r.right()) {
			r.y -= 1;
			r.h += 1;
		}
		while r.bottom() < self.h && self.taken.row_free(r.bottom(), r.x, r.right()) {
			r.h += 1;
		}
		r
	}

	/// Add free rectangles, those another holds left out, and those they hold dropped.
	fn add(&mut self, new: Vec<Rect>) {
		for (i, r) in new.iter().enumerate() {
			// A copy of one before it, or held by another: nothing new.
			if new[..i].contains(r) || self.free.iter().any(|f| f.contains(r)) || new[i + 1..].iter().any(|n| n != r && n.contains(r)) {
				continue;
			}
			self.free.retain(|f| !r.contains(f));
			self.free.push(*r);
		}
	}

	#[cfg(test)]
	pub(crate) fn rects(&self) -> &[Rect] {
		&self.free
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// No free rectangle overlaps a taken unit, and every free unit is in one.
	fn consistent(s: &FreeSpace) {
		for y in 0..s.h {
			for x in 0..s.w {
				let unit = Rect { x, y, w: 1, h: 1 };
				let free = s.taken.row_free(y, x, x + 1);
				let covered = s.free.iter().any(|f| f.contains(&unit));
				assert!(!covered || free, "({x}, {y}) is taken but in a free rectangle");
				assert!(covered || !free, "({x}, {y}) is free but in no free rectangle");
			}
		}
		for (i, a) in s.free.iter().enumerate() {
			for (j, b) in s.free.iter().enumerate() {
				assert!(i == j || !a.contains(b), "{b:?} is within {a:?}");
			}
		}
	}

	#[test]
	fn a_region_goes_where_it_fits_best() {
		let mut s = FreeSpace::new(10, 10);
		s.take(Rect { x: 0, y: 0, w: 7, h: 4 });
		consistent(&s);
		// The 3-wide strip on the right fits a 3-wide region exactly.
		let (fit, x, y) = s.best(3, 2).unwrap();
		assert_eq!((fit, x, y), ((0, 8), 7, 0));
		assert_eq!(s.best(11, 1), None);
	}

	#[test]
	fn given_back_space_is_whole_again() {
		let mut s = FreeSpace::new(8, 8);
		let regions = [
			Rect { x: 0, y: 0, w: 4, h: 4 },
			Rect { x: 4, y: 0, w: 4, h: 4 },
			Rect { x: 0, y: 4, w: 4, h: 4 },
			Rect { x: 4, y: 4, w: 4, h: 4 },
		];
		for r in regions {
			s.take(r);
			consistent(&s);
		}
		assert!(s.rects().is_empty());
		s.give_back(regions[0]);
		s.give_back(regions[1]);
		consistent(&s);
		assert_eq!(s.best(8, 4).map(|(_, x, y)| (x, y)), Some((0, 0)), "the top half, whole");
		s.give_back(regions[3]);
		consistent(&s);
		assert!(s.best(4, 8).is_some(), "the right half");
	}

	/// Regions taken and given back in a scrambled order keep the free space exact.
	#[test]
	fn the_free_space_stays_exact() {
		let mut s = FreeSpace::new(32, 32);
		let mut placed = Vec::new();
		let mut seed = 12345u32;
		let mut next = |n: u32| {
			seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
			(seed >> 16) % n
		};
		for step in 0..400 {
			if next(3) > 0 || placed.is_empty() {
				let (w, h) = (1 + next(8), 1 + next(8));
				if let Some((_, x, y)) = s.best(w, h) {
					let r = Rect { x, y, w, h };
					s.take(r);
					placed.push(r);
				}
			} else {
				let r = placed.swap_remove(next(placed.len() as u32) as usize);
				s.give_back(r);
			}
			if step % 20 == 0 {
				consistent(&s);
			}
		}
		consistent(&s);
	}
}
