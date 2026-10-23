//! When frames start: the game steps once per refresh of the display, as the original
//! does (it presents every frame, and VSync holds each back to the display's pace).
//! Here a frame that would show what the last did (see [`crate::shown`]) is neither
//! drawn nor presented, so the display can't hold it back: the window does, on a frame
//! clock that keeps the display's pace.
//!
//! **The frame clock.** Each frame stands for a refresh of the display, its slot. A
//! frame left undrawn takes the slot one refresh after the last frame's, and the next
//! frame is due a refresh after that, whenever the frame itself started: the clock runs
//! on its slots, so a timer's lateness doesn't add up, and a frame presented after
//! undrawn ones comes a whole refresh after the last. Waiting a refresh after a frame's
//! own start instead, a presented frame that followed undrawn ones (late, out of step
//! with the display) wasn't held back by the display, whose queue had room, and the
//! next frame came at once: with the UI page's paints landing every other frame (a CSS
//! fade over the whole page at 2560x1358), drawn and undrawn frames alternated and the
//! scripts stepped ~312 times a second on a 180 Hz display, ~114 on a 60 Hz one.
//!
//! A frame presented after a presented one is due a little sooner than a refresh after
//! its slot (by [`LEAD`]): over a run of presented frames the clock gains on the
//! display until the display holds the frames back (its queue full, the swapchain's
//! next image waits for a refresh), and from then on the display paces them, as it
//! paces the original's. Paced by the clock alone, a clock slower than the display by
//! the error of its measure would leave the display a refresh without a new frame now
//! and then. A frame that starts later than its slot's due time (held back by the
//! display, or late from its own work) takes its own start as its slot: the clock never
//! catches up on frames.
//!
//! **Where the compositor paces redraws** ([`Pacer::compositor_paces`], Wayland), the
//! clock doesn't hold back the frame after a presented one at all: the system holds its
//! redraw until the compositor has taken the presented frame (the frame callback), a
//! refresh later, whether the frame is then presented or not. Held back by the clock
//! too, such a frame waited for both, one after the other, and its start said nothing
//! certain of either (see below).
//!
//! **The display's refresh** is measured from presented frames, robustly: an interval
//! counts only well into a run of frames presented one after another (the first few
//! after a stretch of undrawn frames go out as fast as the swapchain has images free,
//! not at the display's pace), only between two frames the clock didn't hold back, and
//! the pace is the median of the recent ones (a frame late now and then, or early,
//! doesn't set it). A frame the clock held back started when the clock said, however
//! late its timer woke it (the system's load): its interval is the clock's, never less
//! than the measured refresh, so measuring it fed the measure back into itself. Under
//! load, the timers late, more than half the intervals came in long, the measure grew,
//! and no later interval could be shorter than it: on gamescope's Wayland at 180 Hz
//! (each frame held back by the clock, then by the frame callback) it climbed 5.57,
//! 5.95, 6.26, 6.49, 7.02, 7.27, 7.91, 8.00 ms in a load burst and stayed there, a
//! still screen stepping ~116 times a second afterwards. Frames the clock didn't hold
//! back started when the display (or their own work) let them: the measure doesn't
//! bound their intervals, so a load burst lengthens it only while the burst lasts.
//! Until enough are measured, frames are presented whether they changed or not
//! ([`Pacer::measuring`]): the system needn't report the monitor's refresh (gamescope's
//! Wayland doesn't), and a guess (60 Hz) would step the scripts at a third of a 180 Hz
//! display's rate. Taking the shortest instead let one short early interval set the
//! pace for as long as nothing more was presented: the scripts stepped at ~470 a second
//! on a 180 Hz display. Where the system reports the monitor's refresh, the pace is
//! never faster than that.
//!
//! **A stale measure is noticed and taken again.** Paced by a clock slower than the
//! display (the refresh measured too long), a run of presented frames is never held
//! back by the display, so no interval the display set is measured again: the measure
//! could only grow, from frames late from their own work. (Where the compositor paces
//! redraws, the clock holds no frame after a presented one back, so a run of presented
//! frames keeps the display's pace and measures it whatever was measured before.) A
//! heavy battle's 25 ms frames, slow boot frames, or a measure taken on a 60 Hz monitor
//! before the window moved to a 180 Hz one then kept the game at 40 or 60 steps a
//! second for good. With a measure right (or a little long, by less than [`LEAD`]'s
//! share), the clock gains on the display and it holds a run of presented frames back
//! within about `2 * LEAD` frames (the two frames in flight); so a run of [`STALE`]
//! presented frames that the display never held back, while it could have, shows the
//! measure is too long, and it is forgotten and taken again (frames presented at once
//! until it is). The display holding frames back shows as two frames in a row the clock
//! didn't hold, no more than a refresh apart: one alone, or two further apart, is as
//! likely late from its own work (a frame or two now and then that take longer than a
//! refresh, a loaded system's periodic bursts, would otherwise keep a stale measure for
//! good). A change in the monitor's reported refresh takes it again too. (Not a new
//! video mode, nor the first report as the window shows: those come in the boot, whose
//! heavy frames measured a refresh a few percent long, which then held while too few
//! frames were presented in a row to notice it: 172.7 steps a second at 180 Hz with 2 of
//! 7 frames presented.)
//!
//! **Where the system's report is the display's pace** ([`Pacer::trust_reported`],
//! macOS), the reported refresh is the clock's and nothing is measured. There a redraw
//! asked for comes with the window's next display cycle, so every frame starts a little
//! after its due time: none was ever "started by the clock", each interval counted as
//! the display's, and on a 60 Hz display the measure settled at 22-26 ms (38-47 steps a
//! second, with 1-2 ms of work a frame, measured on a Mac), never noticed as stale.
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
/// takes its own start as its slot, not its due time: the clock doesn't catch up on
/// the lateness (a timer's wake-up is late by far less, but for a loaded system).
const LATE: u32 = 8;
/// How many presented frames in a row, each held back by the clock and none held back by
/// the display while it could hold them, show the measured refresh is too long: with it
/// right, the clock gains a refresh on the display every [`LEAD`] frames, and with two
/// frames in flight the display holds them back within about twice that.
const STALE: u32 = 4 * LEAD;

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
	/// Whether the reported refresh is the display's pace, not measured (see
	/// [`Pacer::trust_reported`]).
	trusted: bool,
	last: Option<Slot>,
	last_presented: Option<Instant>,
	/// The frames presented one after another so far, the last included.
	run: u32,
	/// The presented frames in a row that the clock didn't hold back, the last included.
	unclocked: u32,
	/// The presented frames in a row that the clock started, the display holding none
	/// of them back while it could (see [`STALE`]).
	unheld: u32,
	/// Whether the system holds the redraw after a present for the compositor (see
	/// [`Pacer::compositor_paces`]).
	compositor_paced: bool,
	/// How many times the refresh was measured again.
	#[cfg(test)]
	remeasured: u32,
	intervals: VecDeque<Duration>,
}

impl Pacer {
	pub fn new(reported: Option<Duration>, display_paced: bool) -> Self {
		Pacer {
			reported,
			display_paced,
			trusted: false,
			last: None,
			last_presented: None,
			run: 0,
			unclocked: 0,
			unheld: 0,
			compositor_paced: false,
			#[cfg(test)]
			remeasured: 0,
			intervals: VecDeque::with_capacity(KEPT),
		}
	}

	/// The monitor's refresh as the system reports it now (the window may have moved
	/// to another): if it changed from one reported refresh to another, the display's
	/// refresh is measured again. Not when it's first reported (as the window first
	/// shows): measuring again then falls in the boot's heavy frames.
	pub fn set_reported(&mut self, reported: Option<Duration>) {
		let moved = matches!((self.reported, reported), (Some(a), Some(b)) if a != b);
		self.reported = reported;
		if moved {
			self.remeasure();
		}
	}

	/// Take the reported refresh as the display's pace, measuring only while none is
	/// reported: where the system paces the window's redraws itself (macOS, whose
	/// reports come from the display's own mode), so the frames' intervals measure the
	/// redraws' lateness as much as the display.
	pub fn trust_reported(&mut self, trusted: bool) {
		self.trusted = trusted;
	}

	/// Whether the system holds back a redraw asked for after a present until the
	/// compositor has taken the presented frame (Wayland's frame callback): then the
	/// clock leaves the frame after a presented one to it.
	pub fn compositor_paces(&mut self, paced: bool) {
		self.compositor_paced = paced;
	}

	/// Presenting now waits for the display, or not (VSync turned on or off): what was
	/// measured is forgotten.
	pub fn set_display_paced(&mut self, display_paced: bool) {
		if display_paced != self.display_paced {
			self.display_paced = display_paced;
			self.remeasure();
		}
	}

	/// Forget the display's measured refresh and measure it again (the window on another
	/// monitor, or a measure shown to be stale).
	pub fn remeasure(&mut self) {
		self.intervals.clear();
		self.run = 0;
		self.unclocked = 0;
		self.unheld = 0;
		self.last_presented = None;
		#[cfg(test)]
		{
			self.remeasured += 1;
		}
	}

	/// When the next frame is due, given the frame cap's interval (if the frame rate is
	/// limited); `None`: at once (presented frames while measuring, or with VSync off
	/// and no cap; the first frame), or after a presented frame where the compositor
	/// paces it.
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
			// The compositor holds the redraw a refresh.
			(_, true) if self.display_holds(cap) && self.compositor_paced => None,
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
		// On the clock's time: at its due time (or sooner, a redraw the system asked
		// for), not held back by the display or late from its own work, or its timer
		// far late. Where the report is trusted, a held-back frame is the clock's
		// however late the redraw came, up to a refresh (macOS hands it over with the
		// window's display cycle, 1-4 ms after it was asked for): taking its start as
		// its slot would lose that lateness every frame (56.7 steps a second at 60 Hz,
		// 2026-10-21).
		let late = if self.trusted && self.reported.is_some() { interval } else { interval / LATE };
		let clocked = due.is_some_and(|due| at < due || (waited && at < due + late));
		// Its start the clock's, however late: held back to its due time, or a redraw
		// the system asked for before it.
		let held = waited || due.is_some_and(|due| at < due);
		let slot = match (clocked, self.last) {
			// An undrawn frame stands for the refresh after the last frame's.
			(true, Some(last)) if !presented => last.at + interval,
			(true, _) => due.unwrap_or(at),
			(false, _) => at,
		};
		self.last = Some(Slot { at: slot, presented });
		if !presented {
			self.run = 0;
			self.unclocked = 0;
			self.unheld = 0;
			return;
		}
		self.run = match self.last_presented {
			Some(_) if self.run > 0 => self.run + 1,
			_ => 1,
		};
		self.unclocked = if held { 0 } else { self.unclocked + 1 };
		// Two frames in a row the clock didn't hold back: the display set the second's
		// start (or the frames' own work did, frame after frame).
		let displayed = self.unclocked >= 2;
		// ... and no more than a refresh apart, so as the display holding frames back
		// at the measured pace, not slow frames (a loaded system's bursts).
		let since = self.last_presented.map(|last| at.saturating_duration_since(last));
		let held_back = displayed && since.is_some_and(|since| since <= interval + interval / LEAD);
		// A run of presented frames the display could hold back but never did: the
		// clock is slower than the display, the measure too long.
		let trusted = self.trusted && self.reported.is_some();
		if self.run > 1 && !self.measuring() && !trusted && self.display_holds(cap) {
			if held {
				self.unheld += 1;
			} else if held_back {
				self.unheld = 0;
			}
			if self.unheld >= STALE {
				self.remeasure();
				return;
			}
		}
		// An interval the clock set says nothing of the display (see the module's doc).
		if let (Some(last), true) = (self.last_presented, self.run > SETTLING && displayed) {
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
		self.display_paced && self.intervals.len() < ENOUGH && !(self.trusted && self.reported.is_some())
	}

	/// The display's refresh interval: measured, where presenting waits for it, and no
	/// shorter than the monitor's reported one; else as reported; else 60 Hz.
	pub fn refresh(&self) -> Duration {
		let trusted = self.trusted && self.reported.is_some();
		let measured = (self.display_paced && !trusted && self.intervals.len() >= ENOUGH).then(|| self.median()).flatten();
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
