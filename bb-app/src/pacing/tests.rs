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

/// A FIFO display (VSync on): each refresh shows the next frame presented, if one is
/// queued; three swapchain images (two frames in flight), so a frame's image is free once
/// the frame two before it shows. Without frame callbacks (X11, Windows), nothing holds a
/// redraw back for the compositor, so after a present that found room the next frame can
/// start at once; with them (Wayland: gamescope's, as the window tests run), the redraw
/// after a present waits until the presented frame shows.
struct Display {
	refresh: Duration,
	/// The first refresh.
	start: Instant,
	/// When each frame presented shows.
	shown: Vec<Instant>,
	callbacks: bool,
}

impl Display {
	fn new(refresh: Duration, start: Instant) -> Self {
		Display { refresh, start, shown: Vec::new(), callbacks: false }
	}

	/// The same, with frame callbacks or not, which `pacer` is told (see
	/// [`Pacer::compositor_paces`]).
	fn with_callbacks(refresh: Duration, start: Instant, callbacks: bool, pacer: &mut Pacer) -> Self {
		pacer.compositor_paces(callbacks);
		Display { callbacks, ..Display::new(refresh, start) }
	}

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
/// `presented(i)` says changed drawn and presented (all of them while measuring): see
/// [`Window`]. Returns each frame's start.
fn frames(pacer: &mut Pacer, display: &mut Display, secs: f64, presented: impl Fn(usize) -> bool) -> Vec<Instant> {
	let end = display.start + Duration::from_secs_f64(secs);
	Window::new(display).run(pacer, end, presented, |_, _| Duration::ZERO)
}

/// The window making frames on a display: each held back until it's due (its timer 50 µs
/// late), and after a presented frame until that frame shows where the display has
/// frame callbacks; then stepping (frame `i` takes `step(i)`: 0.2 ms), then for a drawn
/// frame the swapchain's image (waiting for one free), and 0.3 ms to draw and present.
struct Window<'a> {
	display: &'a mut Display,
	step: fn(usize) -> Duration,
	/// When the last frame's work was done.
	ready: Instant,
	presented: bool,
	/// The frames so far.
	frames: usize,
}

impl<'a> Window<'a> {
	fn new(display: &'a mut Display) -> Self {
		let ready = display.start;
		Window { display, step: |_| Duration::from_micros(200), ready, presented: false, frames: 0 }
	}

	/// Frames until `end`, those that `presented(i)` says changed (frame `i` of all so far)
	/// drawn and presented; on a loaded system frame `i`, when it would start at `at`,
	/// starts `load(i, at)` later (its thread woken late, by the clock's timer or the
	/// compositor's callback alike). Returns each frame's start.
	fn run(
		&mut self,
		pacer: &mut Pacer,
		end: Instant,
		presented: impl Fn(usize) -> bool,
		load: impl Fn(usize, Instant) -> Duration,
	) -> Vec<Instant> {
		let (draw, timer_late) = (Duration::from_micros(300), Duration::from_micros(50));
		let mut starts = Vec::new();
		loop {
			let i = self.frames;
			let (mut at, waited) = match pacer.due(None) {
				Some(due) if due > self.ready => (due + timer_late, true),
				_ => (self.ready, false),
			};
			if self.display.callbacks && self.presented {
				at = at.max(*self.display.shown.last().unwrap() + timer_late);
			}
			at += load(i, at);
			if at >= end {
				return starts;
			}
			self.presented = presented(i) || pacer.measuring();
			self.ready = at + (self.step)(i);
			if self.presented {
				let acquired = self.display.image_free().map_or(self.ready, |free| free.max(self.ready));
				self.ready = acquired + draw;
				self.display.present(self.ready);
			}
			pacer.frame(at, self.presented, waited, None);
			starts.push(at);
			self.frames += 1;
		}
	}
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
	for (refresh, callbacks) in [(HZ_180, false), (HZ_60, false), (HZ_180, true), (HZ_60, true)] {
		for (name, presented) in patterns {
			let start = Instant::now();
			let mut pacer = Pacer::new(None, true);
			let mut display = Display::with_callbacks(refresh, start + refresh / 3, callbacks, &mut pacer);
			let starts = frames(&mut pacer, &mut display, 4.0, presented);
			let hz = 1.0 / refresh.as_secs_f64();
			let rate = rate(&starts);
			let on = format!("{name} on a {hz:.0} Hz display, callbacks {callbacks}");
			assert!((rate - hz).abs() < hz * 0.005, "{on}: {rate:.1} steps a second");
			// No step comes much sooner than a refresh after the last.
			let shortest = starts[starts.len() / 2..].windows(2).map(|w| w[1] - w[0]).min().unwrap();
			assert!(shortest > refresh * 9 / 10, "{on}: a step {shortest:?} after the last");
		}
	}
}

/// A run of presented frames is paced by the display, as the original's: once the clock
/// has gained on it, the display holds them back, and every refresh shows a new frame
/// (the clock alone, a hair slower than the display, would leave a refresh without one
/// now and then).
#[test]
fn a_run_of_presented_frames_is_paced_by_the_display() {
	for (refresh, callbacks) in [(HZ_180, false), (HZ_60, false), (HZ_180, true), (HZ_60, true)] {
		let start = Instant::now();
		let mut pacer = Pacer::new(None, true);
		let mut display = Display::with_callbacks(refresh, start + refresh / 3, callbacks, &mut pacer);
		// Undrawn frames for about a second, after measuring, then all presented.
		let undrawn = (1.0 / refresh.as_secs_f64()) as usize;
		let starts = frames(&mut pacer, &mut display, 4.0, |i| i >= undrawn);
		let shown = &display.shown[display.shown.len() / 2..];
		assert!(shown.windows(2).all(|w| w[1] - w[0] == refresh), "a refresh without a new frame (callbacks {callbacks})");
		// From the presented run on, a step per refresh, and two more as the display's
		// queue took its two frames in flight (none with frame callbacks).
		let first = starts[undrawn];
		let steps = starts.len() - undrawn;
		let refreshes = (*starts.last().unwrap() - first).as_secs_f64() / refresh.as_secs_f64() + 1.0;
		assert!((steps as f64 - refreshes).abs() < 2.5, "{steps} steps in {refreshes:.1} refreshes (callbacks {callbacks})");
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

/// Frames presented `every` apart from `from`, `n` of them, none held back by the
/// window: frames late from their own work (a heavy battle, slow boot frames).
fn slow_frames(pacer: &mut Pacer, from: Instant, every: Duration, n: u32) -> Instant {
	let mut t = from;
	for _ in 0..n {
		pacer.frame(t, true, false, None);
		t += every;
	}
	t
}

/// The steps a second, a long while after `secs` of busy frames (each one presented) on a
/// 180 Hz display that starts at `from`.
fn busy_rate_at_180(pacer: &mut Pacer, from: Instant, secs: f64) -> f64 {
	let mut display = Display::new(HZ_180, from + HZ_180 / 3);
	rate(&frames(pacer, &mut display, secs, |_| true))
}

/// A heavy stretch (a big battle on a slow machine, 25 ms a frame) set the measured
/// refresh, and the clock, slower than the display then held back no frame, so nothing
/// shorter was measured again: light frames afterwards stepped ~41 times a second on a
/// 180 Hz display, for good.
#[test]
fn a_heavy_stretch_doesnt_slow_the_game_for_good() {
	let start = Instant::now();
	let mut pacer = Pacer::new(None, true);
	assert!((busy_rate_at_180(&mut pacer, start, 1.0) - 180.0).abs() < 2.0);
	let t = slow_frames(&mut pacer, start + Duration::from_secs(1), Duration::from_millis(25), 40);
	assert!(pacer.interval(None) > Duration::from_millis(20), "the heavy frames' pace, while they last");
	let rate = busy_rate_at_180(&mut pacer, t, 4.0);
	assert!((rate - 180.0).abs() < 2.0, "after a heavy stretch: {rate:.1} steps a second, interval {:?}", pacer.interval(None));
}

/// Slow boot frames as the first measure.
#[test]
fn slow_first_frames_dont_set_the_pace_for_good() {
	let start = Instant::now();
	let mut pacer = Pacer::new(None, true);
	let t = slow_frames(&mut pacer, start, Duration::from_millis(25), 10);
	assert!(!pacer.measuring());
	let rate = busy_rate_at_180(&mut pacer, t, 4.0);
	assert!((rate - 180.0).abs() < 2.0, "after slow boot frames: {rate:.1} steps a second, interval {:?}", pacer.interval(None));
}

/// Measured on a 60 Hz monitor, then the window moves to a 180 Hz one (or the mode
/// changes): whether the system reports the new refresh or not.
#[test]
fn a_faster_display_after_measuring_is_followed() {
	for reports in [true, false] {
		let start = Instant::now();
		let mut pacer = Pacer::new(reports.then_some(HZ_60), true);
		let mut display = Display::new(HZ_60, start + HZ_60 / 3);
		let _ = frames(&mut pacer, &mut display, 2.0, |_| true);
		assert_eq!(pacer.interval(None), HZ_60);
		if reports {
			pacer.set_reported(Some(HZ_180));
		}
		let rate = busy_rate_at_180(&mut pacer, start + Duration::from_secs(2), 4.0);
		assert!((rate - 180.0).abs() < 2.0, "reported {reports}: {rate:.1} steps a second, interval {:?}", pacer.interval(None));
	}
}

/// With the measure right, the display holds a run of presented frames back well before
/// [`STALE`] of them: the measure is never taken again for nothing (each time costs a few
/// frames presented at once), whichever frames are presented, nor with a frame cap the
/// display can't hold back (144 a second on a 180 Hz display).
#[test]
fn a_right_measure_is_kept() {
	type Pattern = fn(usize) -> bool;
	let patterns: [(&str, Pattern); 4] =
		[("all", |_| true), ("every other", |i| i % 2 == 0), ("runs of 200", |i| i % 300 < 200), ("none", |_| false)];
	for (refresh, callbacks) in [(HZ_180, false), (HZ_60, false), (HZ_180, true), (HZ_60, true)] {
		for (name, presented) in patterns {
			let start = Instant::now();
			let mut pacer = Pacer::new(None, true);
			let mut display = Display::with_callbacks(refresh, start + refresh / 3, callbacks, &mut pacer);
			let _ = frames(&mut pacer, &mut display, 6.0, presented);
			assert_eq!(pacer.remeasured, 0, "{name} at {refresh:?}, callbacks {callbacks}");
			assert_eq!(pacer.interval(None), refresh, "{name} at {refresh:?}, callbacks {callbacks}");
		}
	}
	// A cap longer than the refresh paces presented frames; the display holds none back.
	let cap = Some(Duration::from_secs_f64(1.0 / 144.0));
	let start = Instant::now();
	let mut pacer = Pacer::new(None, true);
	let mut display = Display::new(HZ_180, start + HZ_180 / 3);
	let _ = frames(&mut pacer, &mut display, 0.5, |_| true);
	let mut t = start + Duration::from_secs(1);
	for _ in 0..1000 {
		let due = pacer.due(cap).unwrap();
		t = due.max(t);
		pacer.frame(t, true, true, cap);
		t += Duration::from_micros(500);
	}
	assert_eq!(pacer.remeasured, 0, "with a 144 cap");
}

/// A pseudo-random fraction in [0, 1) for frame `i` (the same every run).
fn noise(i: usize) -> f64 {
	let x = (i as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
	((x ^ (x >> 29)).wrapping_mul(0xbf58_476d_1ce9_e4e5) >> 11) as f64 / (1u64 << 53) as f64
}

/// The system loaded from `from` to `to` (seconds after `start`): two frames in three
/// start late, by up to 5 ms (their thread woken late, by the timer or the frame
/// callback alike).
fn burst(start: Instant, from: f64, to: f64) -> impl Fn(usize, Instant) -> Duration {
	move |i, at| {
		let t = at.saturating_duration_since(start).as_secs_f64();
		let late = noise(i);
		if (from..to).contains(&t) && late < 2.0 / 3.0 { Duration::from_secs_f64(late * 1.5 * 0.005) } else { Duration::ZERO }
	}
}

/// The measure of the refresh ratcheted up under load and never came back. A frame the
/// clock held back can't start before it's due, a measured refresh after the last: with
/// its interval measured whenever it started late (its timer late under load, or the
/// frame callback after the clock on Wayland), more than half came in long in a load
/// burst, the measure grew, and every interval after was at least as long. On gamescope's
/// Wayland at 180 Hz it climbed to 8.00 ms (5.57, 5.95, 6.26, 6.49, 7.02, 7.27, 7.91) in a
/// burst, and a still screen afterwards stepped ~116 times a second. A load burst may
/// lengthen the measure while it lasts; after it, with the mouse moving (each frame
/// presented), the measure is the display's again, and the still screen steps once a
/// refresh. With frame callbacks or without, the timers' lateness as the load.
#[test]
fn a_load_burst_doesnt_slow_the_game_for_good() {
	for callbacks in [true, false] {
		let start = Instant::now();
		let mut pacer = Pacer::new(None, true);
		let mut display = Display::with_callbacks(HZ_180, start + HZ_180 / 3, callbacks, &mut pacer);
		// The mouse moving: every frame presented, a second calm, a second loaded, half a
		// second calm again; then the mouse still for a second, nothing presented.
		let load = burst(start, 1.0, 2.0);
		let mut window = Window::new(&mut display);
		let mut measures = Vec::new();
		for tenth in 1..=25 {
			let _ = window.run(&mut pacer, start + Duration::from_millis(tenth * 100), |_| true, &load);
			measures.push(pacer.refresh());
		}
		let still = window.run(&mut pacer, start + Duration::from_millis(3_500), |_| false, &load);
		let ms = |d: &Duration| format!("{:.2}", d.as_secs_f64() * 1000.0);
		let measured: Vec<String> = measures.iter().map(ms).collect();
		eprintln!("callbacks {callbacks}: the measure every 0.1 s: {}", measured.join(" "));
		let after = pacer.refresh();
		assert!(after.abs_diff(HZ_180) < Duration::from_micros(20), "callbacks {callbacks}: {} ms after the burst", ms(&after));
		// The still second: a step a refresh.
		let rate = (still.len() - 1) as f64 / still[still.len() - 1].duration_since(still[0]).as_secs_f64();
		assert!((rate - 180.0).abs() < 1.0, "callbacks {callbacks}: {rate:.1} steps a second after the burst");
	}
}

/// A measure too long is noticed though frames now and then are late from their own work
/// (here two in twenty take 9 ms each, as a loaded system's periodic bursts): a frame the
/// clock didn't hold back is no sign that the display held it back, nor two in a row
/// further apart than a refresh. Taken as one, each such frame kept the measure a while
/// longer, for good, and their intervals were measured too.
#[test]
fn a_frame_late_now_and_then_doesnt_keep_a_stale_measure() {
	let start = Instant::now();
	let mut pacer = Pacer::new(None, true);
	let t = slow_frames(&mut pacer, start, Duration::from_millis(8), 20);
	assert_eq!(pacer.refresh(), Duration::from_millis(8));
	let mut display = Display::new(HZ_180, t + HZ_180 / 3);
	let mut window = Window::new(&mut display);
	window.step = |i| Duration::from_micros(if i % 20 >= 18 { 9_000 } else { 200 });
	let starts = window.run(&mut pacer, t + Duration::from_secs(4), |_| true, |_, _| Duration::ZERO);
	assert!(pacer.refresh().abs_diff(HZ_180) < Duration::from_micros(20), "measured {:?}", pacer.refresh());
	// A step a refresh, but for at most a refresh the two slow frames lose each time
	// (the display's queue makes up for some): 171-180 a second, not 9.3 ms's 108.
	let rate = rate(&starts);
	assert!((171.0..180.0).contains(&rate), "{rate:.1} steps a second");
}

/// macOS: redraws come late with the window's display cycle, so presented frames 25 ms
/// apart on a 60 Hz display would measure a 25 ms refresh. With the reported refresh
/// trusted, it is the clock's, and nothing is measured; without a report it's measured.
#[test]
fn a_trusted_report_is_the_pace() {
	let start = Instant::now();
	let mut pacer = Pacer::new(None, true);
	pacer.trust_reported(true);
	assert!(pacer.measuring(), "nothing reported yet");
	pacer.set_reported(Some(HZ_60));
	assert!(!pacer.measuring());
	presenting(&mut pacer, start, &run(0, Duration::from_micros(25_000), 40));
	assert_eq!(pacer.interval(None), HZ_60, "late redraws don't set the pace");
	let mut untrusted = Pacer::new(Some(HZ_60), true);
	presenting(&mut untrusted, start, &run(0, Duration::from_micros(25_000), 40));
	assert_eq!(untrusted.interval(None), Duration::from_micros(25_000), "measured where not trusted");
}

/// macOS: frames held back to their due time start up to a few milliseconds late (the
/// redraw comes with the display cycle); with the report trusted they keep the clock's
/// slots, so the steps keep the display's rate.
#[test]
fn late_redraws_keep_a_trusted_clock() {
	let start = Instant::now();
	let mut pacer = Pacer::new(Some(HZ_60), true);
	pacer.trust_reported(true);
	pacer.frame(start, false, false, None);
	let mut at = start;
	for i in 0..600u64 {
		let due = pacer.due(None).expect("due");
		assert!(due >= at);
		// 1-4 ms late, the frame undrawn or presented.
		at = due + Duration::from_micros(1_000 + (i * 7919) % 3_000);
		pacer.frame(at, i % 3 == 0, true, None);
	}
	let rate = 600.0 / at.duration_since(start).as_secs_f64();
	assert!((59.0..61.0).contains(&rate), "{rate:.2} steps a second");
}
