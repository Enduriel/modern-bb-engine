//! What the page's paints cost on our side, counted as they happen (CEF's paint thread),
//! for measuring the paint path (`page_paint` in bb-app).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The paint path's counters since the page started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintStats {
	/// Paints of the view.
	pub paints: u64,
	/// Time spent in our paint handler (taking the paint into the frame buffer).
	pub handler: Duration,
	/// Bytes the handler copied into the frame buffer.
	pub copied_bytes: u64,
	/// Pixels the paints said had changed (their dirty rectangles' areas).
	pub damaged_pixels: u64,
}

impl PaintStats {
	/// What happened between `earlier` and these.
	pub fn since(&self, earlier: &PaintStats) -> PaintStats {
		PaintStats {
			paints: self.paints - earlier.paints,
			handler: self.handler.saturating_sub(earlier.handler),
			copied_bytes: self.copied_bytes - earlier.copied_bytes,
			damaged_pixels: self.damaged_pixels - earlier.damaged_pixels,
		}
	}
}

/// [`PaintStats`] as counters the paint thread adds to.
#[derive(Default)]
pub(super) struct PaintCounters {
	paints: AtomicU64,
	handler_nanos: AtomicU64,
	copied_bytes: AtomicU64,
	damaged_pixels: AtomicU64,
}

impl PaintCounters {
	pub fn add(&self, handler: Duration, copied_bytes: u64, damaged_pixels: u64) {
		self.paints.fetch_add(1, Ordering::Relaxed);
		self.handler_nanos.fetch_add(handler.as_nanos() as u64, Ordering::Relaxed);
		self.copied_bytes.fetch_add(copied_bytes, Ordering::Relaxed);
		self.damaged_pixels.fetch_add(damaged_pixels, Ordering::Relaxed);
	}

	pub fn get(&self) -> PaintStats {
		PaintStats {
			paints: self.paints.load(Ordering::Relaxed),
			handler: Duration::from_nanos(self.handler_nanos.load(Ordering::Relaxed)),
			copied_bytes: self.copied_bytes.load(Ordering::Relaxed),
			damaged_pixels: self.damaged_pixels.load(Ordering::Relaxed),
		}
	}
}
