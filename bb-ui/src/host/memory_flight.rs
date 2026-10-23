//! A memory release on its way through the page ([`memory`](super::memory)): what it
//! waits on (its DevTools calls' results, the renderer's trim) and how long it took, for
//! the frame times (`page release`) and the log.
//!
//! The release's parts run on the renderer's main thread, as the page's own work does:
//! while they run, the page can't park, so its time is what a frame waiting on the page
//! would wait.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use cef::*;

use super::client::Shared;

/// How long a release counts as on its way at most: a renderer that died meanwhile
/// never answers.
const GIVE_UP: Duration = Duration::from_secs(60);

/// A release that finished: its whole time, from sending it to its last part's answer,
/// and when each part answered.
#[derive(Debug)]
pub struct Released {
	pub total: Duration,
	pub parts: Vec<(&'static str, Duration)>,
}

impl Released {
	/// What the log says of it: its time, and when each part answered.
	pub fn describe(&self) -> String {
		let ms = |d: &Duration| d.as_secs_f64() * 1000.0;
		let parts: Vec<String> = self.parts.iter().map(|(name, at)| format!("{name} {:.1} ms", ms(at))).collect();
		format!("the page gave memory back in {:.1} ms ({})", ms(&self.total), parts.join(", "))
	}
}

/// The release on its way, if any, and those finished since the game last looked.
#[derive(Default)]
pub(super) struct Flights(Mutex<State>);

#[derive(Default)]
struct State {
	flight: Option<Flight>,
	done: Vec<Released>,
}

struct Flight {
	sent: Instant,
	/// The DevTools calls not answered yet: their message ids and names.
	methods: Vec<(i32, &'static str)>,
	/// Whether the renderer's trim hasn't answered yet.
	trim: bool,
	parts: Vec<(&'static str, Duration)>,
}

impl Flights {
	fn state(&self) -> std::sync::MutexGuard<'_, State> {
		self.0.lock().unwrap_or_else(|e| e.into_inner())
	}

	/// Whether a release is on its way (one given up on isn't).
	pub fn in_flight(&self) -> bool {
		let mut state = self.state();
		if state.flight.as_ref().is_some_and(|f| f.sent.elapsed() > GIVE_UP) {
			state.flight = None;
		}
		state.flight.is_some()
	}

	/// A release was sent at `sent`: it waits on the DevTools calls `methods` and, if
	/// `trim`, the renderer's trim.
	pub fn start(&self, sent: Instant, methods: Vec<(i32, &'static str)>, trim: bool) {
		let mut state = self.state();
		let flight = Flight { sent, methods, trim, parts: Vec::new() };
		if flight.methods.is_empty() && !flight.trim {
			state.done.push(Released { total: sent.elapsed(), parts: Vec::new() });
		} else {
			state.flight = Some(flight);
		}
	}

	/// DevTools answered message `id` (any of the browser's calls: those not the
	/// release's are ignored).
	pub fn method_done(&self, id: i32) {
		self.answered(|flight| match flight.methods.iter().position(|(m, _)| *m == id) {
			Some(i) => Some(flight.methods.remove(i).1),
			None => None,
		});
	}

	/// The renderer's trim finished.
	pub fn trim_done(&self) {
		self.answered(|flight| std::mem::take(&mut flight.trim).then_some("trim"));
	}

	/// A part answered, if `take` finds it in the flight: noted, and the flight
	/// finished with its last.
	fn answered(&self, take: impl FnOnce(&mut Flight) -> Option<&'static str>) {
		let mut state = self.state();
		let Some(flight) = state.flight.as_mut() else { return };
		let Some(part) = take(flight) else { return };
		flight.parts.push((part, flight.sent.elapsed()));
		if flight.methods.is_empty() && !flight.trim {
			let flight = state.flight.take().expect("a flight");
			state.done.push(Released { total: flight.sent.elapsed(), parts: flight.parts });
		}
	}

	/// The releases finished since the last call.
	pub fn take_done(&self) -> Vec<Released> {
		std::mem::take(&mut self.state().done)
	}
}

// Passes the DevTools calls' answers on to the release's flight.
wrap_dev_tools_message_observer! {
	pub(super) struct FlightObserver {
		shared: std::sync::Arc<Shared>,
	}

	impl DevToolsMessageObserver {
		fn on_dev_tools_method_result(
			&self,
			_browser: Option<&mut Browser>,
			message_id: ::std::os::raw::c_int,
			_success: ::std::os::raw::c_int,
			_result: Option<&[u8]>,
		) {
			self.shared.releases.method_done(message_id);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_release_finishes_with_its_last_part() {
		let flights = Flights::default();
		flights.start(Instant::now(), vec![(3, "collect"), (4, "pressure")], true);
		assert!(flights.in_flight());
		flights.method_done(4);
		flights.method_done(9);
		flights.trim_done();
		assert!(flights.take_done().is_empty());
		flights.method_done(3);
		let done = flights.take_done();
		assert_eq!(done.len(), 1);
		assert_eq!(done[0].parts.iter().map(|(p, _)| *p).collect::<Vec<_>>(), ["pressure", "trim", "collect"]);
		assert!(!flights.in_flight());
	}

	#[test]
	fn a_release_with_nothing_to_wait_on_is_done_at_once() {
		let flights = Flights::default();
		flights.start(Instant::now(), Vec::new(), false);
		assert!(!flights.in_flight());
		assert_eq!(flights.take_done().len(), 1);
	}
}
