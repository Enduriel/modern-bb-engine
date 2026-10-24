//! How much real time each step of the game is given: while the display paces the
//! frames, whole refreshes of it, so what a frame shows is where things are when the
//! display shows it.
//!
//! A frame's start (the real time since the last one's) isn't when the display shows
//! it. With VSync on, each presented frame is shown at a refresh of the display, a
//! refresh after the last; but its start comes late now and then (the system's timer,
//! the compositor's frame callback, or the last frame's own work), and the display's
//! queue hides that: the frame is still shown on time. Stepped by its start, such a
//! frame moves things too far for the refresh it shows at, and the next too little:
//! on a 180 Hz display, a frame started 1.5 ms late and the next 1.5 ms early. The
//! world map's parties travelling at 8x (~800 px a second) then stepped 1.27 and 0.73
//! times as far as the frames around them (a headless 180 Hz run with a mod corpus
//! after a few in-game weeks), which reads as small jumps however
//! steady the frame rate; at 1x the same jitter is a tenth of a pixel. The window tests
//! pin it (`the_world_steps_whole_refreshes_though_frames_start_late`).
//!
//! The original doesn't do this (probed with a new campaign travelling at 8x in a
//! headless 60 Hz gamescope): its `Time.getDelta` is the time since the last frame's
//! start in whole milliseconds (16 and 17 alternating), with the same late and early
//! pairs (21/14, 20/13, 27/7 in four seconds), and the party's steps follow them. Whole
//! refreshes are a deliberate divergence: what scripts see is
//! real time within a couple of refreshes, as smooth as the display.
//!
//! So while presenting waits for the display and its refresh is known (see
//! `pacing::Pacer::display_refresh`), a step is given the whole refreshes nearest to the
//! real time owed (at least one: each presented frame is shown for at least one), and
//! the rest is carried to the next. A frame the display really showed late (it missed
//! a refresh: a frame that took longer than one) owes two and gets two. A sixteenth of
//! what is carried goes into each step, so a refresh measured a little off doesn't
//! build up into a jump of a whole refresh now and then. The scripts' real time stays
//! within [`RESYNC`] refreshes of the wall clock: past that, the step takes all that is
//! owed and nothing is carried.
//!
//! Each step is its frame's real time (with what was carried), as before, without a known refresh (VSync off, a
//! frame cap slower than the display, the refresh still being measured), and while
//! most recent frames came well off a whole number of refreshes apart ([`OFF_GRID`]):
//! then the display isn't showing them a refresh at a time (a variable-refresh display
//! under frames slower than its fastest refresh shows each as it comes), and a late
//! start now and then (a few in a thousand frames) isn't what is going on.

/// How many refreshes the game's real time may drift from the wall clock before a step
/// takes all that's owed.
const RESYNC: f64 = 2.0;

/// The share of the carried real time each step takes beyond its whole refreshes.
const BLEED: f64 = 1.0 / 16.0;

/// How far from a whole number of refreshes (a share of one) a frame's real time
/// counts as off the display's grid.
const OFF_GRID: f64 = 0.2;

/// How many recent frames are looked at for how many came off the grid: more than half
/// of them, and steps are real time.
const RECENT: u32 = 32;

/// The real time owed to the game's steps.
#[derive(Debug, Default)]
pub struct DisplayClock {
	/// Real time (ms) passed but not yet given to a step; negative when the steps are
	/// ahead of the wall clock.
	carried: f64,
	/// Which of the last [`RECENT`] frames came off the grid, a bit each (the newest
	/// lowest).
	off_grid: u32,
}

impl DisplayClock {
	/// The real time (ms) to step the game by for a frame `real_ms` after the last,
	/// given the display's refresh interval (ms) while it paces the frames.
	pub fn step(&mut self, real_ms: f64, refresh_ms: Option<f64>) -> f64 {
		let owed = std::mem::take(&mut self.carried) + real_ms;
		let Some(refresh) = refresh_ms.filter(|r| r.is_finite() && *r > 0.0) else {
			self.off_grid = 0;
			return owed.max(0.0);
		};
		let refreshes = real_ms / refresh;
		let off = (refreshes - refreshes.round()).abs() > OFF_GRID;
		self.off_grid = (self.off_grid << 1) | u32::from(off);
		if self.off_grid.count_ones() > RECENT / 2 {
			return owed.max(0.0);
		}
		let whole = (owed / refresh).round().max(1.0) * refresh;
		let step = whole + (owed - whole) * BLEED;
		let carried = owed - step;
		if carried.abs() > RESYNC * refresh {
			return owed.max(0.0);
		}
		self.carried = carried;
		step
	}
}

#[cfg(test)]
mod tests;
