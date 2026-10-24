//! Where frames' regions go in pages: rectangle packing in [`UNIT`]-texel steps, so every
//! place is on the boundary a region's mips need (`bb_assets::frames::ALIGN`).
//!
//! Each page keeps its free space as free rectangles ([`super::free_space`]). A region
//! coming in goes where, of all the pages', it fits best (least left over on its shorter
//! side), so it fills a hole another left before it starts a new page; a freed place is
//! free again for any region that fits it, and a page whose last region goes is freed
//! whole. A region larger than a page gets a page of its own size.
//!
//! Packing them all again ([`PageAlloc::packed`]) places them large first, in a few
//! orders, and keeps the one that takes the fewest pages.

use super::free_space::{FreeSpace, Rect};

/// Places are on multiples of this, in both directions.
pub const UNIT: u32 = 8;

/// Where a region is: its page, and its place there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
	pub page: usize,
	pub x: u32,
	pub y: u32,
	pub w: u32,
	pub h: u32,
}

#[derive(Clone, Debug)]
struct Page {
	width: u32,
	height: u32,
	/// `None` for a region's own page (larger than a page).
	space: Option<FreeSpace>,
	/// Regions placed and not freed, and their units.
	live: usize,
	used: u32,
}

/// The pages' layouts.
#[derive(Clone, Debug)]
pub struct PageAlloc {
	size: u32,
	pages: Vec<Option<Page>>,
}

/// `v` texels in whole units.
fn units(v: u32) -> u32 {
	v.max(1).div_ceil(UNIT)
}

impl PageAlloc {
	/// Pages `size` texels square.
	pub fn new(size: u32) -> PageAlloc {
		PageAlloc {
			size,
			pages: Vec::new(),
		}
	}

	/// A page's size, if it's in use.
	pub fn page_size(&self, page: usize) -> Option<(u32, u32)> {
		self.pages.get(page)?.as_ref().map(|p| (p.width, p.height))
	}

	/// The pages in use.
	pub fn pages(&self) -> impl Iterator<Item = usize> + '_ {
		self.pages.iter().enumerate().filter(|(_, p)| p.is_some()).map(|(i, _)| i)
	}

	/// A place for a `w` × `h` region; the page is new if `new_page` comes back true.
	pub fn alloc(&mut self, w: u32, h: u32) -> (Place, bool) {
		let (uw, uh) = (units(w), units(h));
		let (w, h) = (uw * UNIT, uh * UNIT);
		if w > self.size || h > self.size {
			let page = self.free_page_index();
			self.pages[page] = Some(Page {
				width: w,
				height: h,
				space: None,
				live: 1,
				used: uw * uh,
			});
			return (Place { page, x: 0, y: 0, w, h }, true);
		}
		let best = self
			.pages
			.iter()
			.enumerate()
			.filter_map(|(i, p)| Some((i, p.as_ref()?.space.as_ref()?.best(uw, uh)?)))
			.min_by_key(|&(i, (fit, _, _))| (fit, i));
		let (page, new, x, y) = match best {
			Some((page, (_, x, y))) => (page, false, x, y),
			None => {
				let page = self.free_page_index();
				self.pages[page] = Some(Page {
					width: self.size,
					height: self.size,
					space: Some(FreeSpace::new(self.size / UNIT, self.size / UNIT)),
					live: 0,
					used: 0,
				});
				(page, true, 0, 0)
			}
		};
		let p = self.pages[page].as_mut().expect("a page in use");
		p.space.as_mut().expect("a shared page").take(Rect { x, y, w: uw, h: uh });
		p.live += 1;
		p.used += uw * uh;
		(Place { page, x: x * UNIT, y: y * UNIT, w, h }, new)
	}

	/// Give `place` back; true if its page is now empty (and freed).
	pub fn free(&mut self, place: Place) -> bool {
		let Some(page) = self.pages.get_mut(place.page).and_then(Option::as_mut) else { return false };
		page.live -= 1;
		page.used -= (place.w / UNIT) * (place.h / UNIT);
		if page.live == 0 {
			self.pages[place.page] = None;
			return true;
		}
		if let Some(space) = &mut page.space {
			space.give_back(Rect {
				x: place.x / UNIT,
				y: place.y / UNIT,
				w: place.w / UNIT,
				h: place.h / UNIT,
			});
		}
		false
	}

	/// `regions` (`w` × `h` each) packed into fresh pages `size` square: the layout, and
	/// each region's place. Of a few orders, large first, the one that takes the fewest
	/// pages, then leaves its emptiest page emptiest; the first that takes no more pages
	/// than the regions fill.
	pub fn packed(size: u32, regions: &[(u32, u32)]) -> (PageAlloc, Vec<Place>) {
		type Key = fn(u32, u32) -> (u32, u32);
		let orders: [Key; 4] = [
			|w, h| (w * h, w.max(h)),
			|w, h| (w.max(h), w.min(h)),
			|w, h| (h, w),
			|w, h| (w, h),
		];
		let page = size / UNIT;
		let (own, shared) = regions.iter().map(|&(w, h)| (units(w), units(h))).fold((0, 0), |(own, shared), (w, h)| {
			if w > page || h > page { (own + 1, shared) } else { (own, shared + u64::from(w * h)) }
		});
		let least = own + shared.div_ceil(u64::from(page * page)) as usize;
		let mut best: Option<(PageAlloc, Vec<Place>)> = None;
		for key in orders {
			if best.as_ref().is_some_and(|(b, _)| b.pages().count() <= least) {
				break;
			}
			let mut order: Vec<usize> = (0..regions.len()).collect();
			order.sort_by_key(|&i| {
				let (w, h) = (units(regions[i].0), units(regions[i].1));
				(std::cmp::Reverse(key(w, h)), i)
			});
			let mut alloc = PageAlloc::new(size);
			let mut places = vec![Place { page: 0, x: 0, y: 0, w: 0, h: 0 }; regions.len()];
			for i in order {
				places[i] = alloc.alloc(regions[i].0, regions[i].1).0;
			}
			if best.as_ref().is_none_or(|(b, _)| alloc.cost() < b.cost()) {
				best = Some((alloc, places));
			}
		}
		best.unwrap_or_else(|| (PageAlloc::new(size), Vec::new()))
	}

	/// `regions` packed afresh ([`PageAlloc::packed`]), if that takes fewer pages than
	/// these.
	pub fn repacked(&self, regions: &[(u32, u32)]) -> Option<(PageAlloc, Vec<Place>)> {
		let (layout, places) = PageAlloc::packed(self.size, regions);
		(layout.pages().count() < self.pages().count()).then_some((layout, places))
	}

	/// Pages, then what the emptiest holds: less is better (a page nearly empty goes
	/// sooner, or takes the next regions).
	fn cost(&self) -> (usize, u32) {
		let pages = self.pages.iter().flatten();
		(pages.clone().count(), pages.map(|p| p.used).min().unwrap_or(0))
	}

	fn free_page_index(&mut self) -> usize {
		match self.pages.iter().position(Option::is_none) {
			Some(i) => i,
			None => {
				self.pages.push(None);
				self.pages.len() - 1
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn regions_go_on_unit_boundaries() {
		let mut a = PageAlloc::new(64);
		let (p1, new) = a.alloc(10, 16);
		assert!(new);
		assert_eq!(p1, Place { page: 0, x: 0, y: 0, w: 16, h: 16 }, "rounded up to whole units");
		let (p2, new) = a.alloc(16, 13);
		assert!(!new);
		let (p3, _) = a.alloc(8, 32);
		for p in [p1, p2, p3] {
			assert!(p.x % UNIT == 0 && p.y % UNIT == 0);
			assert_eq!(p.page, 0);
		}
		assert!(!overlap(&p1, &p2) && !overlap(&p1, &p3) && !overlap(&p2, &p3));
	}

	fn overlap(a: &Place, b: &Place) -> bool {
		a.page == b.page && a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
	}

	#[test]
	fn freed_places_are_reused_and_an_empty_page_goes() {
		let mut a = PageAlloc::new(32);
		let (p1, _) = a.alloc(16, 32);
		let (p2, _) = a.alloc(16, 32);
		let (p3, new) = a.alloc(16, 16);
		assert!(new && p3.page == 1, "the first page is full: {p3:?}");
		assert!(!a.free(p1), "the page still holds p2");
		let (p4, new) = a.alloc(8, 32);
		assert!(!new);
		assert_eq!((p4.page, p4.x, p4.y), (0, p1.x, 0), "in p1's freed place");
		assert!(!a.free(p2));
		assert!(a.free(p4), "its last region: the page is freed");
		assert_eq!(a.pages().collect::<Vec<_>>(), [1]);
	}

	#[test]
	fn a_region_larger_than_a_page_gets_its_own() {
		let mut a = PageAlloc::new(32);
		let (p, new) = a.alloc(40, 8);
		assert!(new);
		assert_eq!(a.page_size(p.page), Some((40, 8)));
		assert!(a.free(p));
	}

	/// A region coming in fills the hole it fits best, on any page, before a new page.
	#[test]
	fn a_region_fills_a_hole_before_a_new_page() {
		let mut a = PageAlloc::new(32);
		let quarters: Vec<Place> = (0..8).map(|_| a.alloc(16, 16).0).collect();
		assert_eq!(a.pages().count(), 2);
		// A hole on each page: a 16-wide one on the first, an 8-wide one on the second.
		a.free(quarters[1]);
		a.free(quarters[6]);
		let (p, new) = a.alloc(16, 16);
		assert!(!new);
		let (q, new) = a.alloc(8, 16);
		assert!(!new);
		assert!(!overlap(&p, &q));
		for r in &quarters {
			if *r != quarters[1] && *r != quarters[6] {
				assert!(!overlap(r, &p) && !overlap(r, &q));
			}
		}
	}

	#[test]
	fn packed_places_every_region_once() {
		let regions: Vec<(u32, u32)> = (0..60).map(|i| (8 + (i * 37 % 9) * 8, 8 + (i * 53 % 7) * 8)).collect();
		let (alloc, places) = PageAlloc::packed(128, &regions);
		assert_eq!(places.len(), regions.len());
		for (i, p) in places.iter().enumerate() {
			assert!(p.w >= regions[i].0 && p.h >= regions[i].1);
			assert!(alloc.page_size(p.page).is_some());
			for q in &places[..i] {
				assert!(!overlap(p, q), "{p:?} and {q:?}");
			}
		}
	}
}
