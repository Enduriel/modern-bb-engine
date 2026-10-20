//! The frame rate the FPS display shows: the game's frames a second,
//! measured over a quarter of a second at a time.
//!
//! A frame is one turn of the game's loop, stepped and shown. Since a frame that would
//! show just what the last did is left undrawn (the last stays up, paced as the drawn
//! ones are), it counts too: the display shows the rate the game runs at, not how
//! often the picture changes.
//!
//! The display changes only when the rate it shows does, so it doesn't make frames to
//! draw on its own beyond a few a second.

use std::time::{Duration, Instant};

/// How long each measurement lasts: the display changes at most four times a second.
const SPAN: Duration = Duration::from_millis(250);

/// Counts frames and says when the rate shown changes.
#[derive(Debug, Default)]
pub struct FpsMeter {
	/// When the measurement began (the frame before its first), and its frames since.
	since: Option<Instant>,
	frames: u32,
	/// The rate shown last.
	shown: Option<u32>,
}

impl FpsMeter {
	/// A frame started at `now`. Returns the rate to show, when a measurement ends with a
	/// rate other than the one shown.
	pub fn frame(&mut self, now: Instant) -> Option<u32> {
		let Some(since) = self.since else {
			self.since = Some(now);
			return None;
		};
		self.frames += 1;
		let span = now.saturating_duration_since(since);
		if span < SPAN {
			return None;
		}
		// The frames' intervals over the time they took.
		let rate = (f64::from(self.frames) / span.as_secs_f64()).round() as u32;
		self.since = Some(now);
		self.frames = 0;
		(self.shown != Some(rate)).then(|| {
			self.shown = Some(rate);
			rate
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Frames every `interval` from `start` for `count` frames: the rates shown.
	fn run(meter: &mut FpsMeter, start: Instant, interval: Duration, count: u32) -> Vec<u32> {
		(0..count).filter_map(|i| meter.frame(start + interval * i)).collect()
	}

	#[test]
	fn shows_the_rate_and_only_its_changes() {
		let start = Instant::now();
		let mut meter = FpsMeter::default();
		// 180 Hz for two seconds: the rate once, then nothing new.
		let hz180 = Duration::from_nanos(1_000_000_000 / 180);
		assert_eq!(run(&mut meter, start, hz180, 360), vec![180]);
		// Then 60 Hz.
		let start = start + hz180 * 360;
		assert_eq!(run(&mut meter, start, Duration::from_nanos(16_666_667), 120), vec![60]);
	}

	#[test]
	fn a_slow_frame_ends_a_measurement_at_once() {
		let start = Instant::now();
		let mut meter = FpsMeter::default();
		assert_eq!(meter.frame(start), None);
		assert_eq!(meter.frame(start + Duration::from_millis(500)), Some(2));
	}
}
