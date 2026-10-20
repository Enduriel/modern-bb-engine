//! The parts of the page's frame a paint changed: its dirty rectangles clipped to the
//! frame and coalesced, so the owner copies only what changed, in few copies.

/// A rectangle of the page's frame, in its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintRect {
	pub x: u32,
	pub y: u32,
	pub width: u32,
	pub height: u32,
}

impl PaintRect {
	/// The whole of a `width` × `height` frame.
	pub fn whole(width: u32, height: u32) -> PaintRect {
		PaintRect { x: 0, y: 0, width, height }
	}

	/// `(x, y, width, height)` (CEF's signed rectangle) clipped to a `width` × `height`
	/// frame; `None` if nothing of it is inside.
	pub fn clipped(x: i32, y: i32, w: i32, h: i32, width: u32, height: u32) -> Option<PaintRect> {
		let (left, top) = (i64::from(x).max(0), i64::from(y).max(0));
		let right = (i64::from(x) + i64::from(w)).min(i64::from(width));
		let bottom = (i64::from(y) + i64::from(h)).min(i64::from(height));
		(right > left && bottom > top).then(|| PaintRect {
			x: left as u32,
			y: top as u32,
			width: (right - left) as u32,
			height: (bottom - top) as u32,
		})
	}

	fn right(&self) -> u32 {
		self.x + self.width
	}

	fn bottom(&self) -> u32 {
		self.y + self.height
	}

	pub fn area(&self) -> u64 {
		u64::from(self.width) * u64::from(self.height)
	}

	/// Whether the two overlap or share an edge.
	fn touches(&self, other: &PaintRect) -> bool {
		self.x <= other.right() && other.x <= self.right() && self.y <= other.bottom() && other.y <= self.bottom()
	}

	fn union(&self, other: &PaintRect) -> PaintRect {
		let (x, y) = (self.x.min(other.x), self.y.min(other.y));
		PaintRect {
			x,
			y,
			width: self.right().max(other.right()) - x,
			height: self.bottom().max(other.bottom()) - y,
		}
	}
}

/// More separate rectangles than this become one around them all: each is a copy of its
/// own, and by then the page is changing all over.
const MOST_RECTS: usize = 16;

/// The changed parts of a frame, as few rectangles as cover them without copying much
/// that didn't change: rectangles that overlap or touch merge into the one around both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
	rects: Vec<PaintRect>,
}

impl Damage {
	pub fn add(&mut self, rect: PaintRect) {
		if rect.area() == 0 {
			return;
		}
		let mut rect = rect;
		// Merging can make the rectangle touch others it didn't: until nothing touches it.
		while let Some(i) = self.rects.iter().position(|r| r.touches(&rect)) {
			rect = rect.union(&self.rects.swap_remove(i));
		}
		self.rects.push(rect);
		if self.rects.len() > MOST_RECTS {
			let around = self.rects.iter().skip(1).fold(self.rects[0], |a, r| a.union(r));
			self.rects = vec![around];
		}
	}

	pub fn rects(&self) -> &[PaintRect] {
		&self.rects
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn rect(x: u32, y: u32, width: u32, height: u32) -> PaintRect {
		PaintRect { x, y, width, height }
	}

	#[test]
	fn clips_to_the_frame() {
		assert_eq!(PaintRect::clipped(-5, -5, 20, 20, 100, 100), Some(rect(0, 0, 15, 15)));
		assert_eq!(PaintRect::clipped(90, 95, 20, 20, 100, 100), Some(rect(90, 95, 10, 5)));
		assert_eq!(PaintRect::clipped(100, 0, 20, 20, 100, 100), None);
		assert_eq!(PaintRect::clipped(10, 10, 0, 20, 100, 100), None);
		assert_eq!(PaintRect::clipped(i32::MAX, 0, i32::MAX, 1, 100, 100), None);
	}

	#[test]
	fn keeps_apart_what_is_apart_and_merges_what_meets() {
		let mut damage = Damage::default();
		damage.add(rect(0, 0, 10, 10));
		damage.add(rect(50, 50, 10, 10));
		assert_eq!(damage.rects().len(), 2);
		// Overlaps the first.
		damage.add(rect(5, 5, 10, 10));
		// Shares an edge with the second.
		damage.add(rect(60, 50, 5, 10));
		let mut rects = damage.rects().to_vec();
		rects.sort_by_key(|r| r.x);
		assert_eq!(rects, [rect(0, 0, 15, 15), rect(50, 50, 15, 10)]);
		// Bridges both: one around all.
		damage.add(rect(10, 10, 45, 45));
		assert_eq!(damage.rects(), [rect(0, 0, 65, 60)]);
		damage = Damage::default();
		damage.add(rect(3, 3, 0, 9));
		assert!(damage.rects().is_empty(), "an empty rectangle changes nothing");
	}

	#[test]
	fn many_scattered_rects_become_one() {
		let mut damage = Damage::default();
		for i in 0..=MOST_RECTS as u32 {
			damage.add(rect(i * 20, i * 20, 5, 5));
		}
		let n = MOST_RECTS as u32;
		assert_eq!(damage.rects(), [rect(0, 0, n * 20 + 5, n * 20 + 5)]);
	}
}
