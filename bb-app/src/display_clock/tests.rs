use super::*;

const R: f64 = 1000.0 / 180.0;

fn steps(clock: &mut DisplayClock, real: &[f64], refresh: Option<f64>) -> Vec<f64> {
	real.iter().map(|&ms| clock.step(ms, refresh)).collect()
}

/// A frame started late and the next early (the display's queue showing both on time)
/// step a refresh each, as the display shows them: seen at 8x on the world map as
/// 7.05 then 4.07 ms on a 180 Hz display.
#[test]
fn a_late_start_and_an_early_one_step_a_refresh_each() {
	let mut clock = DisplayClock::default();
	let real = [R, R, 7.05, 4.07, R, R, 7.51, 3.59, R, 6.83, 4.28, R, R];
	for (i, step) in steps(&mut clock, &real, Some(R)).into_iter().enumerate() {
		assert!((step - R).abs() < 0.15, "frame {i}: {step:.3} ms, not a refresh ({R:.3})");
	}
}

/// A frame that took longer than a refresh missed one: the display showed the last
/// frame twice, and this one a refresh late, so it steps two.
#[test]
fn a_missed_refresh_steps_two() {
	let mut clock = DisplayClock::default();
	let got = steps(&mut clock, &[R, 11.11, R], Some(R));
	assert!((got[1] - 2.0 * R).abs() < 0.05, "{got:?}");
	assert!((got[2] - R).abs() < 0.05, "{got:?}");
}

/// The steps keep the wall clock's time: over many jittery frames they add up to the
/// real time, within a refresh.
#[test]
fn the_steps_keep_the_wall_clocks_time() {
	let mut clock = DisplayClock::default();
	let mut real_total = 0.0;
	let mut stepped = 0.0;
	for i in 0..10_000u32 {
		// Up to ±1.5 ms of jitter, a late frame's made up by the next (the display shows
		// each a refresh after the last).
		let jitter = f64::from((i / 2).wrapping_mul(2_654_435_761) % 3001) / 1000.0 - 1.5;
		let real = if i % 2 == 0 { R + jitter } else { R - jitter };
		real_total += real;
		stepped += clock.step(real, Some(R));
		assert!((real_total - stepped).abs() <= R, "frame {i}: {real_total:.3} real, {stepped:.3} stepped");
	}
}

/// A refresh measured a little short builds no jump of a whole refresh: what is carried
/// goes into the steps a little at a time.
#[test]
fn a_refresh_measured_a_little_off_never_jumps() {
	let mut clock = DisplayClock::default();
	let measured = R * 0.999;
	for i in 0..100_000 {
		let step = clock.step(R, Some(measured));
		assert!((step - R).abs() < 0.05, "frame {i}: {step:.4} ms");
	}
}

/// Frames that don't keep the refresh (a variable-refresh display slower than its
/// fastest refresh shows each as it comes) keep the wall clock within a couple of
/// refreshes, and once most recent ones came off the grid, step their real time.
#[test]
fn frames_off_the_refresh_step_real_time() {
	let mut clock = DisplayClock::default();
	let (mut real_total, mut stepped) = (0.0, 0.0);
	for i in 0..1000 {
		let real = if i % 3 == 0 { 7.9 } else { 7.0 };
		real_total += real;
		let step = clock.step(real, Some(R));
		stepped += step;
		assert!((real_total - stepped).abs() <= RESYNC * R + 1e-9);
		if i > RECENT {
			assert_eq!(step, real, "frame {i}");
		}
	}
}

/// Without a refresh that paces the frames, each step is its frame's real time; what
/// was carried goes into the first.
#[test]
fn without_a_paced_display_steps_are_real_time() {
	let mut clock = DisplayClock::default();
	let first = clock.step(7.05, Some(R));
	let second = clock.step(4.07, None);
	assert!((first + second - 11.12).abs() < 1e-9, "{first} + {second}");
	assert_eq!(clock.step(3.0, None), 3.0);
	assert!((clock.step(R, Some(R)) - R).abs() < 1e-9);
}

/// A refresh measured 1.5% short (as under load) puts no step of two refreshes among
/// steady frames with late starts: the grid follows the frames.
#[test]
fn a_refresh_measured_short_is_followed() {
	let mut clock = DisplayClock::default();
	let measured = R * 0.985;
	let got: Vec<f64> = (0..2000).map(|i| clock.step(if i % 7 == 6 { R + 2.0 } else if i % 7 == 0 && i > 0 { R - 2.0 } else { R }, Some(measured))).collect();
	for (i, step) in got.iter().enumerate().skip(400) {
		assert!((step - R).abs() < 0.15, "frame {i}: {step:.3} ms");
	}
}

/// A frame started more than half a refresh late (the test's 2 ms late start, and the
/// system's load on top of it) and the next as early, the display's queue showing both a
/// refresh after the last, step a refresh each. Taken as a missed refresh, the late one
/// stepped two and the early one (owed next to nothing) still one: the steps a refresh
/// ahead of the wall clock, given back over the next dozen steps (5.20, 5.22 ... 5.36 ms
/// on a 180 Hz display, a run under load).
#[test]
fn a_start_more_than_half_a_refresh_late_steps_one() {
	for late in [2.8, 3.0, 3.3, 3.8, 4.3] {
		let mut clock = DisplayClock::default();
		let real: Vec<f64> = (0..700).map(|i| if i % 7 == 6 { R + late } else if i % 7 == 0 && i > 0 { R - late } else { R }).collect();
		for (i, step) in steps(&mut clock, &real, Some(R)).into_iter().enumerate() {
			assert!((step - R).abs() < 0.17, "{late} ms late, frame {i}: {step:.3} ms, not a refresh ({R:.3})");
		}
	}
}

/// Frames that missed a refresh, a millisecond early and the next as late, don't pull the
/// grid long: each on-grid frame moves it by how far it came from its whole refreshes,
/// so a late start and the early one it's made up by cancel out. Moved by that per
/// refresh instead, the missed frame's half counted less than the next one's whole, and
/// the grid crept 5% long, starting over from the pacer's refresh again and again.
#[test]
fn missed_refreshes_started_early_keep_the_grid() {
	let mut clock = DisplayClock::default();
	let real: Vec<f64> = (0..3000).map(|i| if i % 7 == 6 { 2.0 * R - 1.0 } else if i % 7 == 0 && i > 0 { R + 1.0 } else { R }).collect();
	for (i, step) in steps(&mut clock, &real, Some(R)).into_iter().enumerate() {
		let whole = (step / R).round();
		assert!((step - whole * R).abs() < 0.05, "frame {i}: {step:.3} ms, not whole refreshes ({R:.3})");
	}
	assert!((clock.grid.unwrap() - R).abs() < 0.01, "the grid {:.3} ms", clock.grid.unwrap());
}
