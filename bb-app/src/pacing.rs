//! When frames start: the game steps once per refresh of the display, as the original
//! does (it presents every frame, and VSync holds each back to the display's pace).
//! Here a frame that would show what the last did (see [`crate::shown`]) is neither
//! drawn nor presented, so the display can't hold it back: the window does, on a frame
//! clock that keeps the display's pace.
//!
//! **The frame clock.** Each frame stands for a refresh of the display, its slot. A frame
//! left undrawn takes the slot one refresh after the last frame's, and the next frame is
//! due a refresh after that, whenever the frame itself started: the clock runs on its
//! slots, so a timer's lateness doesn't add up, and a frame presented after undrawn ones
//! comes a whole refresh after the last. Waiting a refresh after a frame's own start
//! instead, a presented frame that followed undrawn ones (late, out of step with the
//! display) wasn't held back by the display, whose queue had room, and the next frame
//! came at once: with the UI page's paints landing every other frame (a CSS fade over
//! the whole page at 2560x1358), drawn and undrawn frames alternated and the scripts
//! stepped ~312 times a second on a 180 Hz display, ~114 on a 60 Hz one.
//!
//! A frame presented after a presented one is due a little sooner than a refresh after
//! its slot (by [`LEAD`]): over a run of presented frames the clock gains on the display
//! until the display holds the frames back (its queue full, the swapchain's next image
//! waits for a refresh), and from then on the display paces them, as it paces the
//! original's. Paced by the clock alone, a clock slower than the display by the error of
//! its measure would leave the display a refresh without a new frame now and then. A frame
//! that starts later than its slot's due time (held back by the display, or late from
//! its own work) takes its own start as its slot: the clock never catches up on frames.
//!
//! **The display's refresh** is measured from presented frames, robustly: an interval
//! counts only well into a run of frames presented one after another (the first few
//! after a stretch of undrawn frames go out as fast as the swapchain has images free,
//! not at the display's pace), only where the display set it (not the frame clock), and
//! the pace is the median of the recent ones (a frame late now and then, or early,
//! doesn't set it). Until enough are measured, frames are presented whether they changed
//! or not ([`Pacer::measuring`]): the system needn't report the monitor's refresh
//! (gamescope's Wayland doesn't), and a guess (60 Hz) would step the scripts at a third
//! of a 180 Hz display's rate. Taking the shortest instead let one short early interval
//! set the pace for as long as nothing more was presented: the scripts stepped at ~470 a
//! second on a 180 Hz display. Where the system reports the monitor's refresh, the pace
//! is never faster than that.
//!
//! With VSync off (presents don't wait for the display), the frame cap's interval is the
//! clock's (144 by default), or with no cap the display's refresh for undrawn frames and
//! none for presented ones (as fast as they're made).

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
/// How much sooner than a refresh after its slot a frame presented after a presented one
/// is due, as a fraction of the refresh: the clock gains a refresh on the display over
/// this many presented frames, and the display then holds them back. Well above the
/// error of the measured refresh, which is the median of display-paced intervals.
const LEAD: u32 = 32;
/// A frame that started this much of the clock's interval after it was due, or more,
/// wasn't started by the clock (the timer's wake-up is late by far less) but by the
/// display or its own late work.
const LATE: u32 = 8;

/// The last frame on the frame clock.
#[derive(Clone, Copy)]
struct Slot {
	/// The refresh it stands for.
	at: Instant,
	presented: bool,
}

/// The frame clock, and the display's pace measured from the frames presented.
pub struct Pacer {
	/// The monitor's refresh interval as the system reports it, if it does.
	reported: Option<Duration>,
	/// Whether presenting waits for the display (VSync on: FIFO). Otherwise the
	/// presented frames' intervals say nothing of the display.
	display_paced: bool,
	last: Option<Slot>,
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
			last: None,
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

	/// When the next frame is due, given the frame cap's interval (if the frame rate is
	/// limited); `None`: at once (presented frames while measuring, or with VSync off
	/// and no cap; the first frame).
	pub fn due(&self, cap: Option<Duration>) -> Option<Instant> {
		if self.measuring() {
			return None;
		}
		let last = self.last?;
		let interval = self.interval(cap);
		if !last.presented {
			return Some(last.at + interval);
		}
		match (cap, self.display_paced) {
			// The display holds presented frames back once the clock has gained on it.
			(_, true) if self.display_holds(cap) => Some(last.at + interval - interval / LEAD),
			(Some(_), _) => Some(last.at + interval),
			(None, _) => None,
		}
	}

	/// A frame started at `at`, and was presented or not; `waited`: the window held it
	/// back until it was due ([`Pacer::due`], given the same `cap`).
	pub fn frame(&mut self, at: Instant, presented: bool, waited: bool, cap: Option<Duration>) {
		let due = self.due(cap);
		let interval = self.interval(cap);
		// Started by the clock: at its due time (or sooner, a redraw the system asked
		// for), not held back by the display or late from its own work.
		let clocked = due.is_some_and(|due| at < due || (waited && at < due + interval / LATE));
		let slot = match (clocked, self.last) {
			// An undrawn frame stands for the refresh after the last frame's.
			(true, Some(last)) if !presented => last.at + interval,
			(true, _) => due.unwrap_or(at),
			(false, _) => at,
		};
		self.last = Some(Slot { at: slot, presented });
		if !presented {
			self.run = 0;
			return;
		}
		self.run = match self.last_presented {
			Some(_) if self.run > 0 => self.run + 1,
			_ => 1,
		};
		// An interval the clock set says nothing of the display.
		if let (Some(last), true) = (self.last_presented, self.run > SETTLING && !clocked) {
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

	/// The frame clock's interval, given the frame cap's interval (if the frame rate is
	/// limited): the display's refresh with VSync on (or the cap's, if longer); with it
	/// off, the cap's, or with no cap the display's refresh.
	pub fn interval(&self, cap: Option<Duration>) -> Duration {
		match (cap, self.display_paced) {
			(Some(cap), true) => cap.max(self.refresh()),
			(Some(cap), false) => cap.clamp(FASTEST, SLOWEST),
			(None, _) => self.refresh(),
		}
	}

	/// Whether the display can hold presented frames back: presenting waits for it, and
	/// no longer cap paces them slower.
	fn display_holds(&self, cap: Option<Duration>) -> bool {
		self.display_paced && cap.is_none_or(|cap| cap <= self.refresh())
	}

	/// The median of the recent intervals (the shorter middle one of an even count).
	fn median(&self) -> Option<Duration> {
		let mut sorted: Vec<Duration> = self.intervals.iter().copied().collect();
		sorted.sort_unstable();
		sorted.get(sorted.len().saturating_sub(1) / 2).copied()
	}
}

#[cfg(test)]
mod tests;
