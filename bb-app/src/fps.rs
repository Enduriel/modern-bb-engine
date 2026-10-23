//! The frame rate the FPS display shows: the frames presented a
//! second, measured over a quarter of a second at a time, and beside it, when it
//! differs, the game's steps a second: `20 (144)`.
//!
//! A step is one turn of the game's loop. A frame that would show just what the last
//! did is left undrawn and unpresented (the last stays up, paced as the drawn ones are):
//! the step counts, the frame doesn't. So the display shows what the player sees, as an
//! overlay that counts presents (Steam's) does, and a still screen reads 0 or a little
//! more (the display's own changes are presented), or the display's refresh rate while
//! the mouse moves over it (the last frame presented again, `crate::mouse_refresh`). The scripts' `Time.getFPS` stays the
//! step rate.
//!
//! The display changes only when what it shows does, so it doesn't make frames to draw
//! on its own beyond a few a second.

use std::time::{Duration, Instant};

/// How long each measurement lasts: the display changes at most four times a second.
const SPAN: Duration = Duration::from_millis(250);

/// Counts steps and presented frames and says when what the display shows changes.
#[derive(Debug, Default)]
pub struct FpsMeter {
	/// When the measurement began (the step before its first), and its steps and
	/// presented frames since.
	since: Option<Instant>,
	steps: u32,
	presented: u32,
	/// What was shown last.
	shown: Option<String>,
}

impl FpsMeter {
	/// A step started at `now`. Returns what to show, when a measurement ends with
	/// other than what is shown.
	pub fn frame(&mut self, now: Instant) -> Option<String> {
		let Some(since) = self.since else {
			self.since = Some(now);
			return None;
		};
		self.steps += 1;
		let span = now.saturating_duration_since(since);
		if span < SPAN {
			return None;
		}
		// The steps' intervals over the time they took, and the frames presented in it
		// (each after its step).
		let rate = |n: u32| (f64::from(n) / span.as_secs_f64()).round() as u32;
		let (presented, steps) = (rate(self.presented), rate(self.steps));
		let text = if presented == steps { presented.to_string() } else { format!("{presented} ({steps})") };
		self.since = Some(now);
		self.steps = 0;
		self.presented = 0;
		(self.shown.as_ref() != Some(&text)).then(|| {
			self.shown = Some(text.clone());
			text
		})
	}

	/// The step's frame was presented.
	pub fn presented(&mut self) {
		if self.since.is_some() {
			self.presented += 1;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Steps every `interval` from `start` for `count` steps, every `every`th presented
	/// (0: none): what is shown.
	fn run(meter: &mut FpsMeter, start: Instant, interval: Duration, count: u32, every: u32) -> Vec<String> {
		let mut shown = Vec::new();
		for i in 0..count {
			shown.extend(meter.frame(start + interval * i));
			if every > 0 && i % every == 0 {
				meter.presented();
			}
		}
		shown
	}

	#[test]
	fn shows_the_rate_and_only_its_changes() {
		let start = Instant::now();
		let mut meter = FpsMeter::default();
		// 180 Hz for two seconds, each presented: the rate once, then nothing new.
		let hz180 = Duration::from_nanos(1_000_000_000 / 180);
		assert_eq!(run(&mut meter, start, hz180, 360, 1), vec!["180"]);
		// Then 60 Hz: the measurement across the change in between, then 60.
		let start = start + hz180 * 360;
		let shown = run(&mut meter, start, Duration::from_nanos(16_666_667), 120, 1);
		assert_eq!(shown.last().map(String::as_str), Some("60"), "{shown:?}");
		assert!(shown.len() <= 2 && shown.iter().all(|r| (60..180).contains(&r.parse::<u32>().unwrap())), "{shown:?}");
	}

	#[test]
	fn the_steps_show_beside_the_frames_presented_when_they_differ() {
		let start = Instant::now();
		let mut meter = FpsMeter::default();
		// 160 Hz: 40 steps a measurement, one in 4 presented: 40 frames a second.
		let hz160 = Duration::from_micros(6250);
		assert_eq!(run(&mut meter, start, hz160, 320, 4), vec!["40 (160)"]);
		// A still screen: none presented.
		let shown = run(&mut meter, start + hz160 * 320, hz160, 160, 0);
		assert_eq!(shown, vec!["0 (160)"]);
	}

	#[test]
	fn a_slow_frame_ends_a_measurement_at_once() {
		let start = Instant::now();
		let mut meter = FpsMeter::default();
		assert_eq!(meter.frame(start), None);
		meter.presented();
		assert_eq!(meter.frame(start + Duration::from_millis(500)).as_deref(), Some("2"));
	}
}
