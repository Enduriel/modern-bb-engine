//! The page's latest frame as painted: its size and paint count, and what of its alpha
//! the game has read back so far.
//!
//! The frame itself is on the GPU (the owner's [`PageSink`](super::PageSink)). "Is the
//! cursor over the UI" asks for its alpha at a pixel on every mouse move, so the alpha
//! is read back a tile ([`TILE`] pixels square) at a time and kept until a paint changes
//! something in that tile: moving over a still page reads each tile once, and an
//! animation elsewhere on the page doesn't throw away the tiles it doesn't touch.

use std::collections::HashMap;

use super::damage::PaintRect;

/// The side of a tile of alpha read back at once.
pub const TILE: u32 = 64;

/// A copy of the page's latest painted frame.
pub struct FrameBuffer {
	pub width: u32,
	pub height: u32,
	/// BGRA, premultiplied, rows top to bottom.
	pub bgra: Vec<u8>,
	/// How many paints had happened when it was read.
	pub paints: u64,
}

/// A tile's alpha, rows top to bottom (narrower or shorter at the frame's edges).
struct Tile {
	width: u32,
	alpha: Vec<u8>,
}

/// The latest frame's size and paint count, and the alpha tiles read from it.
#[derive(Default)]
pub(super) struct PaintState {
	pub width: u32,
	pub height: u32,
	/// Paints so far.
	pub paints: u64,
	tiles: HashMap<(u32, u32), Tile>,
}

impl PaintState {
	/// A paint of a `width` × `height` frame landed, changing `dirty`: the tiles it
	/// touched (all of them, if the frame changed size) must be read again.
	pub fn painted(&mut self, width: u32, height: u32, dirty: &[PaintRect]) {
		self.paints += 1;
		if (width, height) != (self.width, self.height) {
			self.width = width;
			self.height = height;
			self.tiles.clear();
			return;
		}
		if self.tiles.is_empty() {
			return;
		}
		for rect in dirty {
			let (x0, y0) = (rect.x / TILE, rect.y / TILE);
			let (x1, y1) = ((rect.x + rect.width).div_ceil(TILE), (rect.y + rect.height).div_ceil(TILE));
			for ty in y0..y1 {
				for tx in x0..x1 {
					self.tiles.remove(&(tx, ty));
				}
			}
		}
	}

	/// The kept alpha at `(x, y)` (inside the frame), if its tile was read since it last
	/// changed.
	pub fn alpha(&self, x: u32, y: u32) -> Option<u8> {
		let tile = self.tiles.get(&(x / TILE, y / TILE))?;
		tile.alpha.get(((y % TILE) * tile.width + x % TILE) as usize).copied()
	}

	/// The tile around `(x, y)`, clipped to the frame.
	pub fn tile_at(&self, x: u32, y: u32) -> PaintRect {
		let (left, top) = (x / TILE * TILE, y / TILE * TILE);
		PaintRect {
			x: left,
			y: top,
			width: TILE.min(self.width.saturating_sub(left)),
			height: TILE.min(self.height.saturating_sub(top)),
		}
	}

	/// Keep the tile `region` (as [`tile_at`](Self::tile_at) gave it) read back as
	/// `bgra`.
	pub fn keep(&mut self, region: PaintRect, bgra: &[u8]) {
		if bgra.len() != (region.width * region.height * 4) as usize {
			return;
		}
		let alpha = bgra.chunks_exact(4).map(|p| p[3]).collect();
		self.tiles.insert((region.x / TILE, region.y / TILE), Tile { width: region.width, alpha });
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn rect(x: u32, y: u32, width: u32, height: u32) -> PaintRect {
		PaintRect { x, y, width, height }
	}

	/// A tile `region` whose alpha is its pixels' x + y.
	fn read(region: PaintRect) -> Vec<u8> {
		let mut bgra = Vec::new();
		for y in region.y..region.y + region.height {
			for x in region.x..region.x + region.width {
				bgra.extend_from_slice(&[0, 0, 0, (x + y) as u8]);
			}
		}
		bgra
	}

	#[test]
	fn keeps_tiles_until_a_paint_touches_them() {
		let mut state = PaintState::default();
		state.painted(200, 100, &[rect(0, 0, 200, 100)]);
		assert_eq!(state.alpha(5, 5), None, "nothing read yet");
		let tile = state.tile_at(5, 5);
		assert_eq!(tile, rect(0, 0, 64, 64));
		state.keep(tile, &read(tile));
		assert_eq!(state.alpha(5, 6), Some(11));
		// At the frame's edge the tile is clipped.
		let edge = state.tile_at(199, 99);
		assert_eq!(edge, rect(192, 64, 8, 36));
		state.keep(edge, &read(edge));
		assert_eq!(state.alpha(199, 99), Some((199 + 99) as u8));

		// A paint elsewhere keeps both; one touching the first tile's last column drops it.
		state.painted(200, 100, &[rect(100, 10, 20, 20)]);
		assert_eq!(state.alpha(5, 6), Some(11));
		state.painted(200, 100, &[rect(63, 63, 1, 1)]);
		assert_eq!(state.alpha(5, 6), None);
		assert_eq!(state.alpha(199, 99), Some((199 + 99) as u8));
		assert_eq!(state.paints, 3);

		// Another size: nothing kept.
		state.painted(100, 100, &[rect(0, 0, 1, 1)]);
		assert_eq!(state.alpha(199, 99), None);
	}
}
