//! The pace of frames the window doesn't present. A presented frame is paced by the
//! display (VSync waits for its refresh) or by the frame cap. A frame left undrawn,
//! because it would show what the last did (see [`crate::shown`]), isn't, so the
//! window waits for itself: a refresh of the display, or the cap's interval. The
//! scripts then run at the rate they'd run at if every frame were drawn, and never
//! faster than the display shows frames.
//!
//! The display's refresh is measured from presented frames, robustly: an interval
//! counts only well into a run of frames presented one after another (the first few
//! after a stretch of undrawn frames go out as fast as the swapchain has images free,
//! not at the display's pace), and the pace is the median of the recent ones (a frame
//! late now and then, or early, doesn't set it). Until enough are measured, frames are
//! presented whether they changed or not ([`Pacer::measuring`]): the system needn't
//! report the monitor's refresh (gamescope's Wayland doesn't), and a guess (60 Hz) would
//! step the scripts at a third of a 180 Hz display's rate. Taking the shortest instead let one
//! short early interval set the pace for as long as nothing more was presented: the
//! scripts stepped at ~470 a second on a 180 Hz display. Where the system reports the
//! monitor's refresh, the pace is never faster than that.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How many recent intervals between presented frames the pace is taken from.
const KEPT: usize = 16;
/// How many frames into a run of presented frames their intervals start to count: with
/// up to two frames in flight (`desired_maximum_frame_latency`), the first ones after a
/// pause aren't held back by the display yet.
const SETTLING: u32 = 3;
/// How many intervals are measured before the pace is taken from them.
const ENOUGH: usize = 4;
/// Bounds on the pace: a display's refresh is within them.
const FASTEST: Duration = Duration::from_micros(1_000);
const SLOWEST: Duration = Duration::from_millis(50);
/// The refresh assumed where nothing better is known: 60 Hz.
const UNKNOWN_REFRESH: Duration = Duration::from_nanos(16_666_667);

/// The display's pace, measured from the frames presented.
pub struct Pacer {
	/// The monitor's refresh interval as the system reports it, if it does.
	reported: Option<Duration>,
	/// Whether presenting waits for the display (VSync on: FIFO). Otherwise the
	/// presented frames' intervals say nothing of the display.
	display_paced: bool,
	last_presented: Option<Instant>,
	/// The frames presented one after another so far, the last included.
	run: u32,
	intervals: VecDeque<Duration>,
}

impl Pacer {
	pub fn new(reported: Option<Duration>, display_paced: bool) -> Self {
		Pacer {
			reported,
			display_paced,
			last_presented: None,
			run: 0,
			intervals: VecDeque::with_capacity(KEPT),
		}
	}

	/// The monitor's refresh as the system reports it now (the window may have moved
	/// to another).
	pub fn set_reported(&mut self, reported: Option<Duration>) {
		self.reported = reported;
	}

	/// Presenting now waits for the display, or not (VSync turned on or off): what was
	/// measured is forgotten.
	pub fn set_display_paced(&mut self, display_paced: bool) {
		if display_paced != self.display_paced {
			self.display_paced = display_paced;
			self.intervals.clear();
			self.run = 0;
			self.last_presented = None;
		}
	}

	/// A frame that started at `at` was presented; `after_presented` if the frame
	/// before it was too (otherwise the time between them says nothing of the display).
	pub fn presented(&mut self, at: Instant, after_presented: bool) {
		self.run = match (self.last_presented, after_presented) {
			(Some(_), true) => self.run + 1,
			_ => 1,
		};
		if let (Some(last), true) = (self.last_presented, self.run > SETTLING) {
			if self.intervals.len() == KEPT {
				self.intervals.pop_front();
			}
			self.intervals.push_back(at.duration_since(last));
		}
		self.last_presented = Some(at);
	}

	/// Whether frames should be presented, changed or not, to measure the display's
	/// refresh: presenting waits for the display, and too little of it is measured yet.
	pub fn measuring(&self) -> bool {
		self.display_paced && self.intervals.len() < ENOUGH
	}

	/// The display's refresh interval: measured, where presenting waits for it, and no
	/// shorter than the monitor's reported one; else as reported; else 60 Hz.
	pub fn refresh(&self) -> Duration {
		let measured = (self.display_paced && self.intervals.len() >= ENOUGH).then(|| self.median()).flatten();
		let refresh = match (measured, self.reported) {
			(Some(measured), Some(reported)) => measured.max(reported),
			(Some(measured), None) => measured,
			(None, Some(reported)) => reported,
			(None, None) => UNKNOWN_REFRESH,
		};
		refresh.clamp(FASTEST, SLOWEST)
	}

	/// The time between undrawn frames, given the frame cap's interval (if the frame
	/// rate is limited): the display's refresh with VSync on (or the cap's, if longer);
	/// with it off, the cap's, or with no cap the display's refresh.
	pub fn interval(&self, cap: Option<Duration>) -> Duration {
		match (cap, self.display_paced) {
			(Some(cap), true) => cap.max(self.refresh()),
			(Some(cap), false) => cap.clamp(FASTEST, SLOWEST),
			(None, _) => self.refresh(),
		}
	}

	/// The median of the recent intervals (the shorter middle one of an even count).
	fn median(&self) -> Option<Duration> {
		let mut sorted: Vec<Duration> = self.intervals.iter().copied().collect();
		sorted.sort_unstable();
		sorted.get(sorted.len().saturating_sub(1) / 2).copied()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const HZ_180: Duration = Duration::from_micros(5_556);
	const HZ_60: Duration = Duration::from_micros(16_667);

	/// A pacer with frames presented at these times (in microseconds from a start), each
	/// after a presented one unless the time is negative (then after an undrawn frame).
	fn presenting(pacer: &mut Pacer, start: Instant, times: &[i64]) {
		for &t in times {
			pacer.presented(start + Duration::from_micros(t.unsigned_abs()), t >= 0);
		}
	}

	/// Frames presented at `interval` from `from`, `n` of them, in one run.
	fn run(from: u64, interval: Duration, n: u64) -> Vec<i64> {
		let us = interval.as_micros() as u64;
		(0..n).map(|i| (from + i * us) as i64).collect()
	}

	#[test]
	fn the_pace_is_the_displays_refresh_measured_from_presented_frames() {
		let start = Instant::now();
		let mut pacer = Pacer::new(None, true);
		assert_eq!(pacer.interval(None), UNKNOWN_REFRESH, "60 Hz before anything is known");
		assert!(pacer.measuring());
		presenting(&mut pacer, start, &run(0, HZ_180, SETTLING as u64 + ENOUGH as u64));
		assert!(!pacer.measuring(), "measured once enough frames are presented in a row");
		assert_eq!(pacer.interval(None), HZ_180);
		// A slower display now: the old intervals go.
		presenting(&mut pacer, start, &run(1_000_000, HZ_60, 40));
		assert_eq!(pacer.interval(None), HZ_60);
	}

	/// The case that stepped the scripts at ~470 a second on a 180 Hz display: a frame
	/// presented 2.1 ms after the last, as the first frames after a pause go out, set the
	/// pace while nothing more was presented.
	#[test]
	fn a_short_interval_after_a_pause_doesnt_set_the_pace() {
		let start = Instant::now();
		let mut pacer = Pacer::new(None, true);
		presenting(&mut pacer, start, &run(0, HZ_180, 20));
		// Undrawn frames, then a mouse move presents a few frames, the second 2.1 ms
		// after the first, the third 2.3 ms after that; then nothing is presented again.
		presenting(&mut pacer, start, &[-500_000, 502_100, 504_400]);
		assert_eq!(pacer.interval(None), HZ_180);
		// Even with nothing measured before.
		let mut fresh = Pacer::new(None, true);
		presenting(&mut fresh, start, &[0, 2_100, 4_400, -100_000, 102_000, 104_100]);
		assert!(fresh.measuring(), "nothing measured: frames are presented until it is");
		assert_eq!(fresh.interval(None), UNKNOWN_REFRESH);
	}

	#[test]
	fn a_frame_late_or_early_now_and_then_doesnt_set_the_pace() {
		let start = Instant::now();
		let mut pacer = Pacer::new(None, true);
		let mut times = run(0, HZ_180, 20);
		times[10] -= 3_000; // early: 2.5 ms after the one before, 8.6 before the next
		times[15] += 4_000; // late: 9.6 ms after the one before, 1.6 before the next
		presenting(&mut pacer, start, &times);
		assert_eq!(pacer.interval(None), HZ_180);
	}

	#[test]
	fn never_faster_than_the_monitors_reported_refresh() {
		let start = Instant::now();
		let mut pacer = Pacer::new(Some(HZ_60), true);
		assert_eq!(pacer.interval(None), HZ_60, "as reported, before anything is measured");
		// A compositor that answers faster than the monitor refreshes.
		presenting(&mut pacer, start, &run(0, HZ_180, 20));
		assert_eq!(pacer.interval(None), HZ_60);
		// A display slower than reported (its refresh, or the game, can't keep up): as
		// measured.
		pacer.set_reported(Some(HZ_180));
		presenting(&mut pacer, start, &run(1_000_000, HZ_60, 20));
		assert_eq!(pacer.interval(None), HZ_60);
	}

	#[test]
	fn with_vsync_off_the_cap_paces_or_the_display() {
		let start = Instant::now();
		let cap = Duration::from_secs_f64(1.0 / 144.0);
		let mut pacer = Pacer::new(Some(HZ_180), true);
		presenting(&mut pacer, start, &run(0, HZ_180, 20));
		assert_eq!(pacer.interval(Some(cap)), cap, "VSync on with a longer cap: the cap");
		pacer.set_display_paced(false);
		assert!(!pacer.measuring(), "nothing to measure");
		// Presents that don't wait: as fast as the frames are made.
		presenting(&mut pacer, start, &run(1_000_000, Duration::from_micros(700), 40));
		assert_eq!(pacer.interval(Some(cap)), cap);
		assert_eq!(pacer.interval(None), HZ_180, "unlimited: the display's refresh, as reported");
		pacer.set_reported(None);
		assert_eq!(pacer.interval(None), UNKNOWN_REFRESH);
		// VSync on again: measured afresh.
		pacer.set_display_paced(true);
		assert!(pacer.measuring());
		presenting(&mut pacer, start, &run(2_000_000, HZ_180, 20));
		assert_eq!(pacer.interval(None), HZ_180);
	}

	#[test]
	fn the_pace_stays_within_a_displays_bounds() {
		let start = Instant::now();
		let mut pacer = Pacer::new(Some(Duration::ZERO), true);
		assert_eq!(pacer.interval(None), FASTEST);
		presenting(&mut pacer, start, &run(0, Duration::from_secs(1), 10));
		assert_eq!(pacer.interval(None), SLOWEST);
	}
}
