//! Where frames' regions go in pages: shelf packing in [`UNIT`]-texel steps, so every
//! place is on the boundary a region's mips need (`bb_assets::frames::ALIGN`).
//!
//! A page is cut into shelves, rows as tall as the first region put in them (or a little
//! taller); a region goes into the first shelf of its height class with room, or a new
//! shelf. A freed place is reused by a region that fits it, and a page whose last region
//! goes is freed whole. A region larger than a page gets a page of its own size.

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

#[derive(Debug)]
struct Shelf {
	y: u32,
	height: u32,
	/// The width used from the left.
	used: u32,
	/// Freed spans within `used`: (x, width).
	free: Vec<(u32, u32)>,
}

#[derive(Debug)]
struct Page {
	width: u32,
	height: u32,
	shelves: Vec<Shelf>,
	/// Regions placed and not freed.
	live: usize,
}

/// The pages' layouts.
#[derive(Debug)]
pub struct PageAlloc {
	size: u32,
	pages: Vec<Option<Page>>,
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
		let (w, h) = (w.max(1).div_ceil(UNIT) * UNIT, h.max(1).div_ceil(UNIT) * UNIT);
		if w > self.size || h > self.size {
			let page = self.free_page_index();
			self.pages[page] = Some(Page {
				width: w,
				height: h,
				shelves: Vec::new(),
				live: 1,
			});
			return (Place { page, x: 0, y: 0, w, h }, true);
		}
		for (index, page) in self.pages.iter_mut().enumerate() {
			let Some(page) = page.as_mut().filter(|p| p.width == self.size) else { continue };
			if let Some((x, y)) = page.place(w, h) {
				page.live += 1;
				return (Place { page: index, x, y, w, h }, false);
			}
		}
		let page = self.free_page_index();
		let mut fresh = Page {
			width: self.size,
			height: self.size,
			shelves: Vec::new(),
			live: 1,
		};
		let (x, y) = fresh.place(w, h).expect("a region fits an empty page");
		self.pages[page] = Some(fresh);
		(Place { page, x, y, w, h }, true)
	}

	/// Give `place` back; true if its page is now empty (and freed).
	pub fn free(&mut self, place: Place) -> bool {
		let Some(page) = self.pages.get_mut(place.page).and_then(Option::as_mut) else { return false };
		page.live -= 1;
		if page.live == 0 {
			self.pages[place.page] = None;
			return true;
		}
		if let Some(shelf) = page.shelves.iter_mut().find(|s| s.y == place.y) {
			shelf.free.push((place.x, place.w));
		}
		false
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

impl Page {
	/// A place for a `w` × `h` region (in units), if the page has one.
	fn place(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
		// A shelf of its height class (no more than half again as tall): a freed span,
		// else room on the right.
		for shelf in self.shelves.iter_mut().filter(|s| s.height >= h && s.height <= h + h / 2) {
			if let Some(i) = shelf.free.iter().position(|&(_, fw)| fw >= w) {
				let (x, fw) = shelf.free[i];
				if fw == w {
					shelf.free.swap_remove(i);
				} else {
					shelf.free[i] = (x + w, fw - w);
				}
				return Some((x, shelf.y));
			}
			if self.width - shelf.used >= w {
				let x = shelf.used;
				shelf.used += w;
				return Some((x, shelf.y));
			}
		}
		let top = self.shelves.last().map_or(0, |s| s.y + s.height);
		if self.height - top < h {
			return None;
		}
		self.shelves.push(Shelf {
			y: top,
			height: h,
			used: w,
			free: Vec::new(),
		});
		Some((0, top))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn regions_share_shelves_on_unit_boundaries() {
		let mut a = PageAlloc::new(64);
		let (p1, new) = a.alloc(10, 16);
		assert!(new);
		assert_eq!(p1, Place { page: 0, x: 0, y: 0, w: 16, h: 16 }, "rounded up to whole units");
		let (p2, new) = a.alloc(16, 13);
		assert!(!new);
		assert_eq!((p2.x, p2.y), (16, 0), "the same shelf");
		let (p3, _) = a.alloc(8, 32);
		assert_eq!((p3.x, p3.y), (0, 16), "too tall for that shelf: a new one");
		for p in [p1, p2, p3] {
			assert!(p.x % UNIT == 0 && p.y % UNIT == 0);
		}
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
		assert_eq!((p4.page, p4.x, p4.y), (0, 0, 0), "in p1's freed place");
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
}
