//! The page's latest frame as painted, kept up to date a paint at a time: each paint
//! copies only what it says changed (its dirty rectangles), and adds that to the
//! [`Damage`] its owner takes to update its own copy (the GPU's).

use super::damage::{Damage, PaintRect};

/// The latest painted frame of the page.
pub struct FrameBuffer {
	pub width: u32,
	pub height: u32,
	/// BGRA, rows top to bottom.
	pub bgra: Vec<u8>,
	/// How many paints have happened so far.
	pub paints: u64,
}

/// The frame and what changed in it since its owner last took that.
#[derive(Default)]
pub(super) struct Painted {
	pub frame: Option<FrameBuffer>,
	pub damage: Damage,
}

impl Painted {
	/// Take a paint of the whole `width` × `height` frame, `pixels` (BGRA), of which the
	/// `dirty` rectangles (`x, y, width, height`) changed. Returns the bytes copied.
	pub fn paint(&mut self, pixels: &[u8], width: u32, height: u32, dirty: impl IntoIterator<Item = (i32, i32, i32, i32)>) -> u64 {
		let paints = self.frame.as_ref().map_or(0, |f| f.paints) + 1;
		let row = width as usize * 4;
		match &mut self.frame {
			Some(frame) if (frame.width, frame.height) == (width, height) => {
				frame.paints = paints;
				let mut changed = Damage::default();
				for (x, y, w, h) in dirty {
					if let Some(rect) = PaintRect::clipped(x, y, w, h, width, height) {
						changed.add(rect);
					}
				}
				let mut copied = 0;
				for rect in changed.rects() {
					let (start, len) = (rect.x as usize * 4, rect.width as usize * 4);
					for y in rect.y as usize..(rect.y + rect.height) as usize {
						let at = y * row + start;
						frame.bgra[at..at + len].copy_from_slice(&pixels[at..at + len]);
					}
					copied += rect.area() * 4;
					self.damage.add(*rect);
				}
				copied
			}
			_ => {
				// The first paint, or of a new size: all of it.
				self.frame = Some(FrameBuffer { width, height, bgra: pixels.to_vec(), paints });
				self.damage.clear();
				self.damage.add(PaintRect::whole(width, height));
				pixels.len() as u64
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A `width` × `height` frame whose every byte is `value`.
	fn solid(width: u32, height: u32, value: u8) -> Vec<u8> {
		vec![value; (width * height * 4) as usize]
	}

	#[test]
	fn copies_only_what_changed_and_keeps_the_frame_whole() {
		let mut painted = Painted::default();
		assert_eq!(painted.paint(&solid(8, 4, 1), 8, 4, [(0, 0, 8, 4)]), 8 * 4 * 4);
		assert_eq!(painted.damage.rects(), [PaintRect::whole(8, 4)]);
		painted.damage.clear();

		// The whole new frame is 2s, but only a 2×2 square is said to have changed.
		assert_eq!(painted.paint(&solid(8, 4, 2), 8, 4, [(3, 1, 2, 2)]), 2 * 2 * 4);
		let frame = painted.frame.as_ref().unwrap();
		assert_eq!(frame.paints, 2);
		for y in 0..4 {
			for x in 0..8 {
				let inside = (3..5).contains(&x) && (1..3).contains(&y);
				assert_eq!(frame.bgra[(y * 8 + x) * 4], if inside { 2 } else { 1 }, "({x}, {y})");
			}
		}
		// A dirty rectangle reaching out of the frame is clipped to it; damage adds up
		// over paints until taken.
		painted.paint(&solid(8, 4, 3), 8, 4, [(6, 3, 10, 10)]);
		assert_eq!(painted.damage.rects().len(), 2);
		assert!(painted.damage.rects().contains(&PaintRect { x: 6, y: 3, width: 2, height: 1 }));
	}

	#[test]
	fn a_new_size_takes_the_whole_frame() {
		let mut painted = Painted::default();
		painted.paint(&solid(8, 4, 1), 8, 4, [(0, 0, 8, 4)]);
		painted.damage.clear();
		assert_eq!(painted.paint(&solid(4, 4, 5), 4, 4, [(0, 0, 1, 1)]), 4 * 4 * 4);
		let frame = painted.frame.as_ref().unwrap();
		assert_eq!((frame.width, frame.height, frame.paints), (4, 4, 2));
		assert!(frame.bgra.iter().all(|&b| b == 5));
		assert_eq!(painted.damage.rects(), [PaintRect::whole(4, 4)]);
	}
}
