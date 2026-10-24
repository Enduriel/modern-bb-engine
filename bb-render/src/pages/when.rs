//! When the pages are packed again ([`super::FramePages::compact`]): after a scene
//! discard took frames (the battle's holes don't keep the world's pages), and once a
//! scene when a frame needs a page more.

/// Whether packing is due.
#[derive(Debug, Default)]
pub(crate) struct PackWhen {
	/// Pack before the next draw.
	due: bool,
	/// A page more was made, and packing asked for, since the last scene discard.
	in_scene: bool,
}

impl PackWhen {
	/// A frame took a page more, and `pages` are in use: packing is due, the first time
	/// this scene. A page more where the frames might fit in fewer: they're packed again
	/// before the next draw if that saves a page, so the holes (frames evicted, regions
	/// placed in the order they came, a load's whole world at once) don't add up between
	/// the scene discards that pack them. Once a scene: packed full again, the pages would
	/// take a page more and be packed at every few frames that come in (110 times, 2-5 ms
	/// each, over a modded 12-battle session).
	pub(crate) fn page_added(&mut self, pages: usize) {
		if !self.in_scene && pages > 1 {
			self.due = true;
			self.in_scene = true;
		}
	}

	/// A scene was discarded, frames gone with it if `any_gone`.
	pub(crate) fn discarded(&mut self, any_gone: bool) {
		self.due |= any_gone;
		self.in_scene = false;
	}

	/// Whether to pack now (and not again until asked).
	pub(crate) fn take(&mut self) -> bool {
		std::mem::take(&mut self.due)
	}
}
