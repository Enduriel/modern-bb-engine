use super::*;

const HZ_180: Duration = Duration::from_micros(5_556);
const HZ_60: Duration = Duration::from_micros(16_667);

/// A pacer with frames presented at these times (in microseconds from a start), each
/// after a presented one unless the time is negative (then after an undrawn frame, just
/// before it). None of them was held back by the window.
fn presenting(pacer: &mut Pacer, start: Instant, times: &[i64]) {
	for &t in times {
		let at = start + Duration::from_micros(t.unsigned_abs());
		if t < 0 {
			pacer.frame(at - Duration::from_micros(1), false, false, None);
		}
		pacer.frame(at, true, false, None);
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

/// A FIFO display (VSync on) as gamescope's: each refresh shows the next frame presented,
/// if one is queued; three swapchain images (two frames in flight), so a frame's image is
/// free once the frame two before it shows; and nothing holds a redraw back for the
/// compositor (no frame callback), so after a present that found room the next frame
/// can start at once.
struct Display {
	refresh: Duration,
	/// The first refresh.
	start: Instant,
	/// When each frame presented shows.
	shown: Vec<Instant>,
}

impl Display {
	/// The first refresh at or after `at`.
	fn refresh_after(&self, at: Instant) -> Instant {
		let n = at.saturating_duration_since(self.start).as_nanos().div_ceil(self.refresh.as_nanos());
		self.start + self.refresh * n as u32
	}

	/// A frame's image is free to draw into: once the frame two before it shows.
	fn image_free(&self) -> Option<Instant> {
		self.shown.len().checked_sub(2).map(|i| self.shown[i])
	}

	/// A frame presented at `at` shows at the first refresh after it, and after the
	/// frame before it.
	fn present(&mut self, at: Instant) {
		let after = self.shown.last().map_or(at, |&last| (last + Duration::from_nanos(1)).max(at));
		let shown = self.refresh_after(after);
		self.shown.push(shown);
	}
}

/// Frames as the window makes them, `secs` of them, on `display`, those that
/// `presented(i)` says changed drawn and presented (all of them while measuring): each
/// held back until it's due (its timer 50 µs late), stepping 0.2 ms, then for a drawn
/// frame the swapchain's image (waiting for one free), and 0.3 ms to draw and present.
/// Returns each frame's start.
fn frames(pacer: &mut Pacer, display: &mut Display, secs: f64, presented: impl Fn(usize) -> bool) -> Vec<Instant> {
	let (step, draw, timer_late) = (Duration::from_micros(200), Duration::from_micros(300), Duration::from_micros(50));
	let end = display.start + Duration::from_secs_f64(secs);
	let mut ready = display.start;
	let mut starts = Vec::new();
	for i in 0.. {
		let (at, waited) = match pacer.due(None) {
			Some(due) if due > ready => (due + timer_late, true),
			_ => (ready, false),
		};
		if at >= end {
			break;
		}
		let presented = presented(i) || pacer.measuring();
		ready = at + step;
		if presented {
			let acquired = display.image_free().map_or(ready, |free| free.max(ready));
			ready = acquired + draw;
			display.present(ready);
		}
		pacer.frame(at, presented, waited, None);
		starts.push(at);
	}
	starts
}

/// Steps a second over the second half of a run.
fn rate(starts: &[Instant]) -> f64 {
	let half = &starts[starts.len() / 2..];
	(half.len() - 1) as f64 / half[half.len() - 1].duration_since(half[0]).as_secs_f64()
}

/// The UI page's paints landing every other frame (a CSS fade over the whole page at
/// 2560x1358): the frames presented and those left undrawn alternated, and each frame
/// after a presented one started at once (the display's queue had room), so the
/// scripts stepped ~312 times a second on a 180 Hz display (~114 on a 60 Hz one).
/// Whatever frames are presented among undrawn ones, the game steps once per refresh.
#[test]
fn the_game_steps_once_a_refresh_whichever_frames_are_presented() {
	type Pattern = fn(usize) -> bool;
	let patterns: [(&str, Pattern); 5] = [
		("every other", |i| i % 2 == 0),
		("two of every seven", |i| i % 7 < 2),
		("one of every three", |i| i % 3 == 0),
		("none", |_| false),
		("all", |_| true),
	];
	for refresh in [HZ_180, HZ_60] {
		for (name, presented) in patterns {
			let start = Instant::now();
			let mut pacer = Pacer::new(None, true);
			let mut display = Display { refresh, start: start + refresh / 3, shown: Vec::new() };
			let starts = frames(&mut pacer, &mut display, 4.0, presented);
			let hz = 1.0 / refresh.as_secs_f64();
			let rate = rate(&starts);
			assert!((rate - hz).abs() < hz * 0.005, "{name} on a {hz:.0} Hz display: {rate:.1} steps a second");
			// No step comes much sooner than a refresh after the last.
			let shortest = starts[starts.len() / 2..].windows(2).map(|w| w[1] - w[0]).min().unwrap();
			assert!(shortest > refresh * 9 / 10, "{name} on a {hz:.0} Hz display: a step {shortest:?} after the last");
		}
	}
}

/// A run of presented frames is paced by the display, as the original's: once the clock
/// has gained on it, the display holds them back, and every refresh shows a new frame
/// (the clock alone, a hair slower than the display, would leave a refresh without one
/// now and then).
#[test]
fn a_run_of_presented_frames_is_paced_by_the_display() {
	for refresh in [HZ_180, HZ_60] {
		let start = Instant::now();
		let mut pacer = Pacer::new(None, true);
		let mut display = Display { refresh, start: start + refresh / 3, shown: Vec::new() };
		// Undrawn frames for about a second, after measuring, then all presented.
		let undrawn = (1.0 / refresh.as_secs_f64()) as usize;
		let starts = frames(&mut pacer, &mut display, 4.0, |i| i >= undrawn);
		let shown = &display.shown[display.shown.len() / 2..];
		assert!(shown.windows(2).all(|w| w[1] - w[0] == refresh), "a refresh without a new frame");
		// From the presented run on, a step per refresh, and two more as the display's
		// queue took its two frames in flight.
		let first = starts[undrawn];
		let steps = starts.len() - undrawn;
		let refreshes = (*starts.last().unwrap() - first).as_secs_f64() / refresh.as_secs_f64() + 1.0;
		assert!((steps as f64 - refreshes).abs() < 2.5, "{steps} steps in {refreshes:.1} refreshes");
	}
}

/// With VSync off, the frame cap's interval is the clock's, presented frames or not: its
/// timer's lateness doesn't add up (paced a cap's interval after each frame's own start,
/// 143.2-143.6 frames a second at a cap of 144).
#[test]
fn with_vsync_off_the_game_steps_at_the_cap() {
	let cap = Duration::from_secs_f64(1.0 / 144.0);
	let (step, timer_late) = (Duration::from_micros(700), Duration::from_micros(50));
	for presented in [(|_| true) as fn(usize) -> bool, |i| i % 2 == 0, |_| false] {
		let start = Instant::now();
		let mut pacer = Pacer::new(Some(HZ_180), false);
		let mut ready = start;
		let mut starts = Vec::new();
		for i in 0..1000 {
			let (at, waited) = match pacer.due(Some(cap)) {
				Some(due) if due > ready => (due + timer_late, true),
				_ => (ready, false),
			};
			ready = at + step;
			pacer.frame(at, presented(i), waited, Some(cap));
			starts.push(at);
		}
		let rate = rate(&starts);
		assert!((rate - 144.0).abs() < 0.1, "{rate:.2} frames a second");
	}
}
