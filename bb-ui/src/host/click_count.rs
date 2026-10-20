//! Counting a mouse button's clicks as the OS does, so the page sees double and triple
//! clicks (`dblclick`, `detail` 2 and 3). An offscreen Chromium counts nothing itself:
//! each press and release carries the count it is given.
//!
//! A press continues the previous one's run when it's the same button, soon enough
//! after it and near enough to it ([`DoubleClick`]); anything else starts a new run at
//! 1. Its release carries the same count.

use std::time::{Duration, Instant};

/// How close in time and space two presses must be to count as one run of clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DoubleClick {
	/// The longest time from one press to the next.
	pub time: Duration,
	/// The rectangle, centred on a press, the next press must land in (view pixels).
	pub width: i32,
	pub height: i32,
}

impl DoubleClick {
	/// Windows' defaults (`GetDoubleClickTime` 500 ms; `SM_CXDOUBLECLK` and
	/// `SM_CYDOUBLECLK` 4 px), which the original's players had under Coherent.
	pub const WINDOWS_DEFAULTS: DoubleClick = DoubleClick {
		time: Duration::from_millis(500),
		width: 4,
		height: 4,
	};

	/// The system's settings. For now Windows' defaults everywhere: Linux has no one
	/// system setting (each desktop keeps its own), and reading Windows' or macOS's
	/// (`GetDoubleClickTime`, `NSEvent.doubleClickInterval`) is still to do.
	pub fn system() -> DoubleClick {
		DoubleClick::WINDOWS_DEFAULTS
	}

	/// Whether `b` is in the rectangle centred on `a`.
	fn near(self, a: (i32, i32), b: (i32, i32)) -> bool {
		2 * (a.0 - b.0).abs() <= self.width && 2 * (a.1 - b.1).abs() <= self.height
	}
}

/// The last press: which button, where, when, and its count.
#[derive(Clone, Copy, Debug)]
struct Press<B> {
	button: B,
	at: (i32, i32),
	time: Instant,
	count: i32,
}

/// Counts the clicks of runs of presses (`B` is the button type).
#[derive(Clone, Copy, Debug)]
pub(super) struct ClickCounter<B> {
	settings: DoubleClick,
	last: Option<Press<B>>,
}

impl<B: Copy + PartialEq> ClickCounter<B> {
	pub(super) fn new(settings: DoubleClick) -> Self {
		ClickCounter {
			settings,
			last: None,
		}
	}

	/// `button` went down at `at` at `now`: its click count.
	pub(super) fn press(&mut self, button: B, at: (i32, i32), now: Instant) -> i32 {
		let count = match self.last {
			Some(last)
				if last.button == button
					&& now.saturating_duration_since(last.time) <= self.settings.time
					&& self.settings.near(last.at, at) =>
			{
				last.count + 1
			}
			_ => 1,
		};
		self.last = Some(Press {
			button,
			at,
			time: now,
			count,
		});
		count
	}

	/// `button` went up: the count of its press.
	pub(super) fn release(&self, button: B) -> i32 {
		self.last
			.filter(|last| last.button == button)
			.map_or(1, |last| last.count)
	}

	/// The mouse moved to `at`: further than the rectangle from the last press ends
	/// its run.
	pub(super) fn moved(&mut self, at: (i32, i32)) {
		if self.last.is_some_and(|last| !self.settings.near(last.at, at)) {
			self.last = None;
		}
	}

	/// Start over: the next press counts 1.
	pub(super) fn reset(&mut self) {
		self.last = None;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[derive(Clone, Copy, Debug, PartialEq, Eq)]
	enum Button {
		Left,
		Right,
	}

	fn ms(n: u64) -> Duration {
		Duration::from_millis(n)
	}

	fn counter() -> ClickCounter<Button> {
		ClickCounter::new(DoubleClick::WINDOWS_DEFAULTS)
	}

	#[test]
	fn quick_presses_in_place_count_up() {
		let mut c = counter();
		let t = Instant::now();
		assert_eq!(c.press(Button::Left, (10, 10), t), 1);
		assert_eq!(c.release(Button::Left), 1);
		assert_eq!(c.press(Button::Left, (10, 10), t + ms(150)), 2);
		assert_eq!(c.release(Button::Left), 2);
		assert_eq!(c.press(Button::Left, (10, 10), t + ms(300)), 3);
		assert_eq!(c.release(Button::Left), 3);
	}

	#[test]
	fn the_time_runs_from_the_previous_press() {
		let mut c = counter();
		let t = Instant::now();
		c.press(Button::Left, (0, 0), t);
		assert_eq!(c.press(Button::Left, (0, 0), t + ms(500)), 2);
		assert_eq!(c.press(Button::Left, (0, 0), t + ms(900)), 3);
		// Too late: a new run.
		assert_eq!(c.press(Button::Left, (0, 0), t + ms(1401)), 1);
	}

	#[test]
	fn presses_must_land_within_the_rectangle() {
		let mut c = counter();
		let t = Instant::now();
		c.press(Button::Left, (100, 100), t);
		// 4 px square centred on the press: 2 px either way.
		assert_eq!(c.press(Button::Left, (102, 98), t + ms(10)), 2);
		assert_eq!(c.press(Button::Left, (105, 98), t + ms(20)), 1);
		// The rectangle is centred on the latest press.
		assert_eq!(c.press(Button::Left, (106, 100), t + ms(30)), 2);
	}

	#[test]
	fn another_button_starts_a_new_run() {
		let mut c = counter();
		let t = Instant::now();
		c.press(Button::Left, (0, 0), t);
		assert_eq!(c.press(Button::Right, (0, 0), t + ms(10)), 1);
		assert_eq!(c.release(Button::Left), 1);
		assert_eq!(c.release(Button::Right), 1);
		assert_eq!(c.press(Button::Left, (0, 0), t + ms(20)), 1);
		assert_eq!(c.press(Button::Left, (0, 0), t + ms(30)), 2);
	}

	#[test]
	fn moving_away_ends_the_run() {
		let mut c = counter();
		let t = Instant::now();
		c.press(Button::Left, (50, 50), t);
		c.moved((51, 52));
		assert_eq!(c.press(Button::Left, (50, 50), t + ms(10)), 2);
		// Out and back between presses.
		c.moved((60, 50));
		c.moved((50, 50));
		assert_eq!(c.press(Button::Left, (50, 50), t + ms(20)), 1);
		c.reset();
		assert_eq!(c.press(Button::Left, (50, 50), t + ms(30)), 1);
	}
}
