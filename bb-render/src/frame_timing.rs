//! Where a frame's time goes, by named span, for profiling the frame (`bb`'s
//! `BB_FRAME_TIMES`). Off until [`enable`]d on the thread, and then a span costs two
//! clock reads.

use std::cell::RefCell;
use std::time::{Duration, Instant};

thread_local! {
	/// The spans' time since the last [`take`], by name in first-seen order (`None`: off).
	static SPANS: RefCell<Option<Vec<(&'static str, Duration)>>> = const { RefCell::new(None) };
}

/// Time spans on this thread from now on.
pub fn enable() {
	SPANS.with(|s| s.borrow_mut().get_or_insert_with(Vec::new).clear());
}

/// Whether spans are timed on this thread.
pub fn enabled() -> bool {
	SPANS.with(|s| s.borrow().is_some())
}

/// A span named `name` until the guard drops (nothing while off). Spans of one name add up.
pub fn span(name: &'static str) -> Span {
	Span(enabled().then(|| (name, Instant::now())))
}

/// Add `time` to the span `name`.
pub fn add(name: &'static str, time: Duration) {
	SPANS.with(|s| {
		if let Some(spans) = s.borrow_mut().as_mut() {
			match spans.iter_mut().find(|(n, _)| *n == name) {
				Some((_, t)) => *t += time,
				None => spans.push((name, time)),
			}
		}
	});
}

/// The spans' time since the last take, and start over.
pub fn take() -> Vec<(&'static str, Duration)> {
	SPANS.with(|s| s.borrow_mut().as_mut().map(std::mem::take).unwrap_or_default())
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
		assert!(take().is_empty());
		enable();
		drop(span("a"));
		add("b", Duration::from_millis(2));
		add("b", Duration::from_millis(3));
		let spans = take();
		assert_eq!(spans.iter().map(|(n, _)| *n).collect::<Vec<_>>(), ["a", "b"]);
		assert_eq!(spans[1].1, Duration::from_millis(5));
		assert!(take().is_empty(), "taken");
	}
}
