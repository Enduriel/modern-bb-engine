//! The game's start, phase by phase: with `BB_STARTUP_TIMES=1` every [`mark`] prints
//! `startup: <phase> <ms>` to stderr, the milliseconds since the process started (the
//! kernel's start time on Linux, so the time before `main` counts too; elsewhere since
//! the first mark). Off, a mark costs one relaxed atomic load.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// 0: not looked up yet, 1: off, 2: on.
static STATE: AtomicU8 = AtomicU8::new(0);
/// The first mark's instant, and how long the process had run by then.
static ORIGIN: OnceLock<(Instant, Duration)> = OnceLock::new();

fn on() -> bool {
	match STATE.load(Ordering::Relaxed) {
		1 => false,
		2 => true,
		_ => {
			let on = std::env::var_os("BB_STARTUP_TIMES").is_some_and(|v| v != "0");
			STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
			on
		}
	}
}

/// How long the process has run, from the kernel's records (Linux: its start time in
/// clock ticks since boot against the system's uptime, both at 10 ms resolution).
#[cfg(target_os = "linux")]
fn process_age() -> Option<Duration> {
	let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
	// The fields after the command's name (which may hold spaces), from the 3rd on.
	let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
	let start_ticks: f64 = fields.get(19)?.parse().ok()?;
	let uptime: f64 = std::fs::read_to_string("/proc/uptime").ok()?.split_whitespace().next()?.parse().ok()?;
	// USER_HZ is 100 on every Linux the game runs on.
	Some(Duration::from_secs_f64((uptime - start_ticks / 100.0).max(0.0)))
}

#[cfg(not(target_os = "linux"))]
fn process_age() -> Option<Duration> {
	None
}

/// The time since the process started, if startup times are on.
pub fn elapsed() -> Option<Duration> {
	if !on() {
		return None;
	}
	let (origin, age) = ORIGIN.get_or_init(|| (Instant::now(), process_age().unwrap_or_default()));
	Some(*age + origin.elapsed())
}

/// The start reached `phase`: printed with the time since the process started, if
/// `BB_STARTUP_TIMES` is on.
pub fn mark(phase: &str) {
	if let Some(t) = elapsed() {
		eprintln!("startup: {phase} {:.0}", t.as_secs_f64() * 1000.0);
	}
}
