//! The pace of frames the window doesn't present. A presented frame is paced by the
//! display (VSync waits for its refresh) or by the frame cap. A frame left undrawn,
//! because it would show what the last did (see [`crate::shown`]), isn't, so the
//! window waits for itself: as long as presenting took of late. The scripts then run
//! at the rate they'd run at if every frame were drawn.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How many recent intervals between presented frames the pace is taken from.
const KEPT: usize = 16;
/// Bounds on the pace: a display's refresh is within them.
const FASTEST: Duration = Duration::from_micros(1_000);
const SLOWEST: Duration = Duration::from_millis(50);

/// The intervals between consecutive presented frames.
pub struct Pacer {
	/// The pace before any frames were presented (the monitor's refresh, as reported).
	fallback: Duration,
	last_presented: Option<Instant>,
	intervals: VecDeque<Duration>,
}

impl Pacer {
	pub fn new(fallback: Duration) -> Self {
		Pacer {
			fallback: fallback.clamp(FASTEST, SLOWEST),
			last_presented: None,
			intervals: VecDeque::with_capacity(KEPT),
		}
	}

	/// A frame that started at `at` was presented; `after_presented` if the frame
	/// before it was too (otherwise the time between them says nothing of the display).
	pub fn presented(&mut self, at: Instant, after_presented: bool) {
		if let (Some(last), true) = (self.last_presented, after_presented) {
			if self.intervals.len() == KEPT {
				self.intervals.pop_front();
			}
			self.intervals.push_back(at.duration_since(last));
		}
		self.last_presented = Some(at);
	}

	/// The time between frames: the shortest of the recent intervals between presented
	/// frames (the display's refresh, a frame being late now and then), within bounds.
	pub fn interval(&self) -> Duration {
		self.intervals.iter().min().copied().unwrap_or(self.fallback).clamp(FASTEST, SLOWEST)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_pace_is_the_shortest_recent_interval_between_presented_frames() {
		let start = Instant::now();
		let ms = |n: u64| start + Duration::from_micros(n * 100);
		let mut pacer = Pacer::new(Duration::from_micros(16_667));
		assert_eq!(pacer.interval(), Duration::from_micros(16_667), "the monitor's, at first");
		pacer.presented(ms(0), false);
		pacer.presented(ms(56), true);
		pacer.presented(ms(167), true); // a late one
		assert_eq!(pacer.interval(), Duration::from_micros(5_600));
		// After an undrawn frame, the next presented one's interval doesn't count.
		pacer.presented(ms(1000), false);
		assert_eq!(pacer.interval(), Duration::from_micros(5_600));
		// The old intervals go: a slower display now.
		for i in 1..=KEPT as u64 {
			pacer.presented(ms(1000 + i * 167), true);
		}
		assert_eq!(pacer.interval(), Duration::from_micros(16_700));
	}

	#[test]
	fn the_pace_stays_within_a_displays_bounds() {
		let start = Instant::now();
		let mut pacer = Pacer::new(Duration::ZERO);
		assert_eq!(pacer.interval(), FASTEST);
		pacer.presented(start, false);
		pacer.presented(start + Duration::from_secs(1), true);
		assert_eq!(pacer.interval(), SLOWEST);
	}
}
