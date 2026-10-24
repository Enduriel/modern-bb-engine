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
//! real time owed (for a frame off the grid, the fewer: see below; at least one: each
//! presented frame is shown for at least one), and the rest is carried to the next. The
//! refresh is the pacer's, followed (within
//! [`FOLLOW`]) towards what the frames on its grid keep, as a measure taken under load
//! can be a little off: each frame on the grid moves it by a [`TRACK`]th of how far the
//! frame came from its whole refreshes (not of that per refresh, which counted a missed
//! frame's lateness half as much as the next frame's earliness that made up for it, and
//! the grid crept long). A frame the display really showed late (it missed a refresh: a
//! frame that took longer than one) owes two and gets two.
//!
//! A frame off the grid gets only the whole refreshes it surely owes, the rest carried:
//! its start alone doesn't tell a frame that started late, which the display still shows
//! a refresh after the last, from one that missed a refresh and started early; the next
//! frame does (early: the two owe a refresh each; on time: what was carried makes up a
//! refresh more). Rounded to the nearest instead, a start more than half a refresh late
//! stepped two refreshes and the early frame after it (owing next to nothing) still one,
//! as each presented frame is shown for one: the steps a whole refresh ahead of the wall
//! clock, given back a sixteenth at a time over the next dozen steps (5.20, 5.22 ... 5.36
//! ms on a 180 Hz display: a 2 ms late start that the system's load made 3.4 ms, the
//! world at 8x then travelling up to 6% slow for a dozen frames).
//!
//! A sixteenth of what was carried in (as far as it is still owed) goes into each step,
//! so a refresh measured a little off doesn't build up into a jump of a whole refresh
//! now and then; what the frame itself brings is left for the next to settle (a late
//! start, made up by an early one), not bled into its own step. The scripts' real time
//! stays within [`RESYNC`] refreshes of the wall clock: past that, the step takes all
//! that is owed and nothing is carried.
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

/// The share of the real time carried in that each step takes beyond its whole refreshes.
const BLEED: f64 = 1.0 / 16.0;

/// How far from a whole number of refreshes (a share of one) a frame's real time
/// counts as off the display's grid.
const OFF_GRID: f64 = 0.2;

/// How many recent frames are looked at for how many came off the grid: more than half
/// of them, and steps are real time.
const RECENT: u32 = 32;

/// How far (a share of it) the grid may follow the frames away from the refresh the
/// pacer gives: further, and the grid starts over from that refresh.
const FOLLOW: f64 = 0.05;

/// The share of the difference between an on-grid frame's real time and its whole
/// refreshes of the grid that the grid moves by.
const TRACK: f64 = 1.0 / 64.0;

/// The real time owed to the game's steps.
#[derive(Debug, Default)]
pub struct DisplayClock {
	/// Real time (ms) passed but not yet given to a step; negative when the steps are
	/// ahead of the wall clock.
	carried: f64,
	/// Which of the last [`RECENT`] frames came off the grid, a bit each (the newest
	/// lowest).
	off_grid: u32,
	/// The grid (ms): the refresh the pacer gives, followed towards what on-grid frames
	/// keep (a refresh measured 1.5% short under load put a step of two refreshes in
	/// every 15 or so).
	grid: Option<f64>,
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
		let grid = match self.grid {
			Some(grid) if (grid / refresh - 1.0).abs() <= FOLLOW => grid,
			_ => refresh,
		};
		let refreshes = real_ms / grid;
		let off = refreshes.round() < 1.0 || (refreshes - refreshes.round()).abs() > OFF_GRID;
		self.off_grid = (self.off_grid << 1) | u32::from(off);
		self.grid = Some(if off { grid } else { grid + (real_ms - refreshes.round() * grid) * TRACK });
		if self.off_grid.count_ones() > RECENT / 2 {
			return owed.max(0.0);
		}
		let refreshes = owed / grid;
		let whole = if off { refreshes.floor() } else { refreshes.round() }.max(1.0) * grid;
		let rest = owed - whole;
		let step = whole + rest.signum() * rest.abs().min((owed - real_ms).abs()) * BLEED;
		let carried = owed - step;
		if carried.abs() > RESYNC * grid {
			return owed.max(0.0);
		}
		self.carried = carried;
		step
	}
}

#[cfg(test)]
mod tests;
