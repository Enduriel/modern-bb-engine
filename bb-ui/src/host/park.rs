//! The park handshake (see the module notes of [`super`]): the game asks the page to
//! park, and waits until it has, or, for a game loop that must go on drawing, only a
//! while ([`UiHost::try_park`]): a park not answered in time stays asked, and later
//! calls wait on it, never on a second one. A park answered late, or not at all within
//! a call's time, is a warning in log.html.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::ipc::{self, Arg};
use super::{CALL_TIMEOUT, Event, RequestKind, Serve, UiHost, recv};
#[cfg(target_os = "macos")]
use super::mac;

/// How long a park may take before it is logged (the frame's scripts waited on it).
const SLOW_PARK: Duration = Duration::from_secs(1);

/// Whether `event` answers the park `id`: the page parked, or it has no bridge yet.
fn park_answer(event: &Event, id: i32) -> bool {
	match event {
		Event::Request { kind: RequestKind::Park, .. } => true,
		Event::Reply { id: reply, .. } => *reply == id,
		_ => false,
	}
}

/// [`recv`], or with no time `left` a look at what arrived already.
fn recv_or_poll<T>(rx: &mpsc::Receiver<T>, left: Duration) -> Option<T> {
	if left.is_zero() {
		#[cfg(target_os = "macos")]
		mac::pump();
		return rx.try_recv().ok();
	}
	recv(rx, left)
}

impl UiHost {
	/// `event`, unless it answers the park on its way ([`try_park`](Self::try_park)):
	/// then the page is parked (or has no bridge yet), and nothing is left of it.
	pub(super) fn take_park_answer(&self, event: Event) -> Option<Event> {
		let Some((id, sent)) = self.park_sent.get() else { return Some(event) };
		if !park_answer(&event, id) {
			return Some(event);
		}
		if let Event::Request { responder, body, .. } = event {
			self.rest.reported_text(&body, Instant::now());
			self.pending.replace(Some(responder));
			self.parked.set(true);
		}
		self.parked_after(sent);
		None
	}

	/// Park the page (see the module notes): it waits in a request the game answers
	/// when it [`unpark`](Self::unpark)s it, and the game's calls into it meanwhile
	/// travel as that request's answers. Its own tasks (animation frames, timers,
	/// input) don't run until then. Waits for the page to finish what it's doing,
	/// serving its calls into the game meanwhile, up to [`CALL_TIMEOUT`] after the park
	/// was asked for: a park still on its way from an earlier frame
	/// ([`try_park`](Self::try_park)) isn't waited on afresh. Nothing to do if the page
	/// is parked already, or blocked in a call the game is serving.
	pub fn park(&self, serve: &mut Serve) -> Result<(), String> {
		if self.park_within(CALL_TIMEOUT, serve)? {
			return Ok(());
		}
		let waited = self.park_sent.get().map_or(Duration::ZERO, |(_, sent)| sent.elapsed());
		Err(format!("the page did not park within {:.3} s", waited.as_secs_f64()))
	}

	/// As [`park`](Self::park), but waits only until `wait` after the park was asked
	/// for (at least a look at what arrived), and says whether the page parked. One that
	/// didn't stays asked: a later call waits on the same park, never on a second one.
	/// For a game loop that must go on drawing while the page is busy (the memory
	/// release at a discard runs on its main thread, a swapping machine can stall it
	/// for seconds): the frame's scripts wait until the page parks, the window doesn't.
	pub fn try_park(&self, wait: Duration, serve: &mut Serve) -> Result<bool, String> {
		self.park_within(wait, serve)
	}

	/// Park the page, waiting until `wait` after the park was asked for (a park on its
	/// way is asked already) or `wait` after the last call of the page's the game
	/// served meanwhile. Whether it parked.
	fn park_within(&self, wait: Duration, serve: &mut Serve) -> Result<bool, String> {
		if self.parked.get() || self.pending.borrow().is_some() {
			return Ok(true);
		}
		let (id, sent) = match self.park_sent.get() {
			Some(asked) => asked,
			None => {
				let id = self.next_id();
				self.send(ipc::PARK, &[Arg::Int(id)])?;
				let asked = (id, Instant::now());
				self.park_sent.set(Some(asked));
				asked
			}
		};
		// A park's answer set aside while something else was waited on.
		let set_aside = {
			let mut deferred = self.deferred.borrow_mut();
			let at = deferred.iter().position(|e| park_answer(e, id));
			at.and_then(|i| deferred.remove(i))
		};
		let mut deadline = (sent + wait).max(Instant::now());
		let mut next = set_aside;
		loop {
			let event = match next.take() {
				Some(event) => event,
				None => {
					let left = deadline.saturating_duration_since(Instant::now());
					match recv_or_poll(&self.events, left) {
						Some(event) => event,
						None => {
							self.note_late_park(sent);
							return Ok(false);
						}
					}
				}
			};
			// Parked, or no bridge in the page yet: none of its scripts run.
			if park_answer(&event, id) {
				self.take_park_answer(event);
				return Ok(true);
			}
			match event {
				Event::Request {
					kind: RequestKind::Call | RequestKind::Task,
					body,
					responder,
				} => {
					self.serve(responder, &body, serve);
					deadline = Instant::now() + wait;
				}
				Event::RendererGone(why) => {
					self.park_sent.set(None);
					let error = format!("the page crashed: {why}");
					self.deferred
						.borrow_mut()
						.push_back(Event::RendererGone(why));
					return Err(error);
				}
				other => self.deferred.borrow_mut().push_back(other),
			}
		}
	}

	/// The park asked for at `sent` was answered: none on its way now. One that took
	/// long is logged (the game's scripts waited on it).
	pub(super) fn parked_after(&self, sent: Instant) {
		self.park_sent.set(None);
		let waited = sent.elapsed();
		if waited >= SLOW_PARK {
			self.warn(&format!("the page parked {:.3} s after it was asked to (it was busy)", waited.as_secs_f64()));
		}
		self.late_park_noted.set(false);
	}

	/// The park asked for at `sent` hasn't been answered: logged once it is
	/// [`CALL_TIMEOUT`] late.
	fn note_late_park(&self, sent: Instant) {
		let waited = sent.elapsed();
		if waited >= CALL_TIMEOUT && !self.late_park_noted.replace(true) {
			self.warn(&format!("the page has not parked {:.3} s after it was asked to (it is busy)", waited.as_secs_f64()));
		}
	}
}
