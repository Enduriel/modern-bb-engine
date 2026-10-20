//! What the page's paints cost on our side, counted as they happen (CEF's paint thread),
//! for measuring the paint path (`page_paint` in bb-app).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The paint path's counters since the page started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintStats {
	/// Paints of the view.
	pub paints: u64,
	/// Time spent in our paint handler (the owner's sink: importing the paint's GPU
	/// buffer and copying what changed, waiting for the GPU).
	pub handler: Duration,
	/// Pixels the paints said had changed (their dirty rectangles' areas, merged).
	pub damaged_pixels: u64,
	/// Paints the sink couldn't take.
	pub failures: u64,
}

impl PaintStats {
	/// What happened between `earlier` and these.
	pub fn since(&self, earlier: &PaintStats) -> PaintStats {
		PaintStats {
			paints: self.paints - earlier.paints,
			handler: self.handler.saturating_sub(earlier.handler),
			damaged_pixels: self.damaged_pixels - earlier.damaged_pixels,
			failures: self.failures - earlier.failures,
		}
	}
}

/// [`PaintStats`] as counters the paint thread adds to.
#[derive(Default)]
pub(super) struct PaintCounters {
	paints: AtomicU64,
	handler_nanos: AtomicU64,
	damaged_pixels: AtomicU64,
	failures: AtomicU64,
}

impl PaintCounters {
	pub fn add(&self, handler: Duration, damaged_pixels: u64) {
		self.paints.fetch_add(1, Ordering::Relaxed);
		self.handler_nanos.fetch_add(handler.as_nanos() as u64, Ordering::Relaxed);
		self.damaged_pixels.fetch_add(damaged_pixels, Ordering::Relaxed);
	}

	/// A paint the sink couldn't take; how many so far.
	pub fn failed(&self) -> u64 {
		self.failures.fetch_add(1, Ordering::Relaxed) + 1
	}

	pub fn get(&self) -> PaintStats {
		PaintStats {
			paints: self.paints.load(Ordering::Relaxed),
			handler: Duration::from_nanos(self.handler_nanos.load(Ordering::Relaxed)),
			damaged_pixels: self.damaged_pixels.load(Ordering::Relaxed),
			failures: self.failures.load(Ordering::Relaxed),
		}
	}
}
