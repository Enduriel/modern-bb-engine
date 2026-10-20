//! When the page makes its frames: on Chromium's own clock, or in step with the game's.
//!
//! A frame is when the page runs its animation frames (`requestAnimationFrame`: jQuery's
//! and Velocity's ticks), advances CSS animations and transitions, and paints what
//! changed. Animations keep real time either way (the frame's time is the clock's), so
//! how often frames come only changes how smoothly they move.
//!
//! The game in a window ([`PageFrames::Game`]) begins one page frame after each frame it
//! draws, so the page keeps up with any refresh rate (the original's UI advanced with
//! its game's frames too). The frame's paint then reaches the next game frame: beginning
//! it after the draw gives the page a whole frame's time to paint, where beginning it
//! with the frame's step made a paint land sometimes before the draw and sometimes
//! after (15% of frames at 180 Hz showed none or two; `page_paint` in bb-app). Nothing
//! changed, nothing is painted: an idle page costs no more than on its own clock.
//!
//! A session waiting on the page with no game frames drawn (its pumps: tests and tools
//! clicking through screens as they animate) begins the frames itself, at most
//! `waiting_rate` a second; otherwise the page would stand still while it waits.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// When the page makes its frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageFrames {
	/// Chromium's own clock, at most this many frames a second (CEF 154 takes any rate:
	/// 1000 was measured).
	Clock(i32),
	/// The game's: one after each frame the game draws ([`UiHost::begin_frame`]), and
	/// while a session waits on the page without drawing, at most `waiting_rate` a
	/// second ([`UiHost::pace`]).
	///
	/// [`UiHost::begin_frame`]: super::UiHost::begin_frame
	/// [`UiHost::pace`]: super::UiHost::pace
	Game { waiting_rate: i32 },
}

impl PageFrames {
	/// Whether the host begins the page's frames (CEF's external begin frames).
	pub(super) fn external(self) -> bool {
		matches!(self, PageFrames::Game { .. })
	}

	/// The rate CEF's own clock runs at. With external frames there is no clock, but the
	/// rate still caps how often a paint is handed over: CEF captures the GPU-painted page
	/// at most this often (its capturer's minimum period), so a game at 180 Hz got 60
	/// paints a second at 60. So the game's frames alone set the pace there.
	pub(super) fn clock_rate(self) -> i32 {
		match self {
			PageFrames::Clock(rate) => rate.max(1),
			PageFrames::Game { .. } => GAME_PAINT_CAP,
		}
	}
}

/// The most paints a second handed over with the game's frames ([`PageFrames::clock_rate`]):
/// above any display's refresh rate (CEF 154 takes any rate).
const GAME_PAINT_CAP: i32 = 1000;

/// When the host last began a page frame, for pacing them while a session waits.
pub(super) struct FrameClock {
	frames: PageFrames,
	last: Cell<Option<Instant>>,
}

impl FrameClock {
	pub fn new(frames: PageFrames) -> FrameClock {
		FrameClock { frames, last: Cell::new(None) }
	}

	pub fn external(&self) -> bool {
		self.frames.external()
	}

	/// A frame begins at `now`.
	pub fn began(&self, now: Instant) {
		self.last.set(Some(now));
	}

	/// Whether a waiting session should begin a frame at `now`: with the host beginning
	/// them, when the last began at least a waiting frame's time ago.
	pub fn due(&self, now: Instant) -> bool {
		let PageFrames::Game { waiting_rate } = self.frames else { return false };
		let interval = Duration::from_secs(1) / waiting_rate.max(1) as u32;
		self.last.get().is_none_or(|last| now.duration_since(last) >= interval)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_waiting_session_paces_only_the_game_s_frames() {
		let now = Instant::now();
		let clock = FrameClock::new(PageFrames::Clock(60));
		assert!(!clock.external() && !clock.due(now), "Chromium's clock needs no help");

		let clock = FrameClock::new(PageFrames::Game { waiting_rate: 50 });
		assert!(clock.external());
		assert!(clock.due(now), "none began yet");
		clock.began(now);
		assert!(!clock.due(now + Duration::from_millis(19)));
		assert!(clock.due(now + Duration::from_millis(20)));
	}

	#[test]
	fn the_clock_rate_is_at_least_one() {
		assert_eq!(PageFrames::Clock(180).clock_rate(), 180);
		assert_eq!(PageFrames::Clock(0).clock_rate(), 1);
		assert_eq!(PageFrames::Game { waiting_rate: 60 }.clock_rate(), GAME_PAINT_CAP, "the game's frames set the pace");
	}
}
