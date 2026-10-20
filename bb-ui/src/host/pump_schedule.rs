//! When CEF wants its work run, for a host whose UI thread is the game's (macOS, see
//! `mac`): CEF says so through `on_schedule_message_pump_work`, from any thread, and the
//! game runs the work (`UiHost::pump`) at that time, woken for it if it is waiting.
//! Without it, the page's paints and its own calls into the game waited for the game's
//! next frame.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Wakes the game's loop (e.g. a winit event loop proxy). Called from any thread.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// The process's schedule: CEF starts once per process, and its handler has no other
/// way to the host.
pub(super) static SCHEDULE: PumpSchedule = PumpSchedule::new();

pub struct PumpSchedule {
	/// When CEF's work is due next, if it asked.
	due: Mutex<Option<Instant>>,
	waker: Mutex<Option<Waker>>,
}

impl PumpSchedule {
	const fn new() -> PumpSchedule {
		PumpSchedule { due: Mutex::new(None), waker: Mutex::new(None) }
	}

	/// CEF asks for its work in `delay_ms` (0 or less: now). Only an earlier time than the
	/// one already asked for wakes the loop, so a busy page doesn't wake it once per task.
	pub(super) fn schedule(&self, delay_ms: i64) {
		let at = Instant::now() + Duration::from_millis(delay_ms.max(0) as u64);
		let earlier = {
			let mut due = self.due.lock().unwrap_or_else(|e| e.into_inner());
			let earlier = due.is_none_or(|d| at < d);
			if earlier {
				*due = Some(at);
			}
			earlier
		};
		if earlier {
			let waker = self.waker.lock().unwrap_or_else(|e| e.into_inner()).clone();
			if let Some(wake) = waker {
				wake();
			}
		}
	}

	/// When CEF's work is due, if it asked.
	pub fn due(&self) -> Option<Instant> {
		*self.due.lock().unwrap_or_else(|e| e.into_inner())
	}

	/// Whether the work is due by `now`; if so it is taken (CEF asks again as it needs).
	#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
	pub(super) fn take_due(&self, now: Instant) -> bool {
		let mut due = self.due.lock().unwrap_or_else(|e| e.into_inner());
		if due.is_some_and(|d| d <= now) {
			*due = None;
			true
		} else {
			false
		}
	}

	/// Forget what was asked: the work was just run anyway.
	#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
	pub(super) fn clear(&self) {
		*self.due.lock().unwrap_or_else(|e| e.into_inner()) = None;
	}

	pub(super) fn set_waker(&self, waker: Option<Waker>) {
		*self.waker.lock().unwrap_or_else(|e| e.into_inner()) = waker;
	}
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicUsize, Ordering};

	use super::*;

	#[test]
	fn only_an_earlier_time_wakes_and_due_work_is_taken_once() {
		let schedule = PumpSchedule::new();
		let wakes = Arc::new(AtomicUsize::new(0));
		let w = wakes.clone();
		schedule.set_waker(Some(Arc::new(move || {
			w.fetch_add(1, Ordering::Relaxed);
		})));
		schedule.schedule(1000);
		schedule.schedule(2000);
		assert_eq!(wakes.load(Ordering::Relaxed), 1, "a later time doesn't wake");
		schedule.schedule(0);
		assert_eq!(wakes.load(Ordering::Relaxed), 2, "an earlier one does");
		assert!(schedule.take_due(Instant::now()));
		assert!(!schedule.take_due(Instant::now()), "taken");
		assert_eq!(schedule.due(), None);
	}
}
