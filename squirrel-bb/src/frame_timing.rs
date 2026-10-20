//! Where a frame's time goes, by named span, for profiling the frame (`bb`'s
//! `BB_FRAME_TIMES`). Off until [`enable`]d on the thread, and then a span costs two
//! clock reads. Here rather than in the renderer so the engine's frame (its scripts,
//! passes and the page's bridge) can be timed too.
//!
//! Spans nest freely: a span's time includes the spans run inside it. Counters
//! ([`count`]) say how often something happened in the frame (calls over the bridge).

use std::cell::RefCell;
use std::time::{Duration, Instant};

/// What's been recorded on this thread since the last [`take`].
#[derive(Default)]
struct Record {
	/// The spans' time, by name in first-seen order.
	spans: Vec<(&'static str, Duration)>,
	/// The counters, by name in first-seen order.
	counts: Vec<(&'static str, u64)>,
}

thread_local! {
	/// `None`: off.
	static RECORD: RefCell<Option<Record>> = const { RefCell::new(None) };
}

/// Time spans on this thread from now on.
pub fn enable() {
	RECORD.with(|r| *r.borrow_mut() = Some(Record::default()));
}

/// Whether spans are timed on this thread.
pub fn enabled() -> bool {
	RECORD.with(|r| r.borrow().is_some())
}

/// A span named `name` until the guard drops (nothing while off). Spans of one name add up.
pub fn span(name: &'static str) -> Span {
	Span(enabled().then(|| (name, Instant::now())))
}

/// Add `time` to the span `name`.
pub fn add(name: &'static str, time: Duration) {
	RECORD.with(|r| {
		if let Some(record) = r.borrow_mut().as_mut() {
			match record.spans.iter_mut().find(|(n, _)| *n == name) {
				Some((_, t)) => *t += time,
				None => record.spans.push((name, time)),
			}
		}
	});
}

/// Add `n` to the counter `name`.
pub fn count(name: &'static str, n: u64) {
	RECORD.with(|r| {
		if let Some(record) = r.borrow_mut().as_mut() {
			match record.counts.iter_mut().find(|(c, _)| *c == name) {
				Some((_, total)) => *total += n,
				None => record.counts.push((name, n)),
			}
		}
	});
}

/// The spans' time since the last take, and start over.
pub fn take() -> Vec<(&'static str, Duration)> {
	RECORD.with(|r| r.borrow_mut().as_mut().map(|r| std::mem::take(&mut r.spans)).unwrap_or_default())
}

/// The counters since the last take, and start over.
pub fn take_counts() -> Vec<(&'static str, u64)> {
	RECORD.with(|r| r.borrow_mut().as_mut().map(|r| std::mem::take(&mut r.counts)).unwrap_or_default())
}

/// A running span (see [`span`]).
#[must_use]
pub struct Span(Option<(&'static str, Instant)>);

impl Drop for Span {
	fn drop(&mut self) {
		if let Some((name, since)) = self.0 {
			add(name, since.elapsed());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn spans_add_up_by_name_only_while_enabled() {
		drop(span("off"));
		count("off", 1);
		assert!(take().is_empty());
		assert!(take_counts().is_empty());
		enable();
		drop(span("a"));
		add("b", Duration::from_millis(2));
		add("b", Duration::from_millis(3));
		let spans = take();
		assert_eq!(spans.iter().map(|(n, _)| *n).collect::<Vec<_>>(), ["a", "b"]);
		assert_eq!(spans[1].1, Duration::from_millis(5));
		assert!(take().is_empty(), "taken");
	}

	#[test]
	fn counters_add_up_by_name() {
		enable();
		count("calls", 2);
		count("parks", 1);
		count("calls", 3);
		assert_eq!(take_counts(), [("calls", 5), ("parks", 1)]);
		assert!(take_counts().is_empty(), "taken");
	}
}
