//! The original's key queue (its input manager: queueing `0x5517f0`, dispatch
//! `0x551550`; disassembled 2026-10-14). A key's press stays queued and reaches the
//! states again every frame until its release takes it out, so the scripts' held-key
//! handlers (the world map's and the battle's WASD scrolling: `1500 · Time.getDelta()`
//! a call) run once a frame, from the first frame, not at the system's key-repeat
//! rate. The system's repeats of a held key are dropped. Backspace and Tab (codes 37
//! and 38) are dispatched once, held or not.
//!
//! A key event's `getState()` and `getFire()` are both whether the key is down (the
//! original fills both fields with it).
//!
//! Nothing comes up when the window loses the keyboard: the original only notes it
//! (`0x5b57bd`), so a key held then stays held until its next release.
//!
//! What a key types goes through the queue too (`0x5b68e0`, key `0x62` with the
//! character), after its key, each of the system's repeats again: the UI gets it and
//! the states don't (`0x522600` skips it). The UI's listener comes first and never
//! keeps an event from the states (`0x56cb40`), so the page gets a held key's keydown
//! every frame, as the states get its press (probed 2026-10-21: 60 keydowns in a
//! second's hold, the typed character at the system's repeat rate).

use std::cell::RefCell;

use crate::input_events::KeyInput;

/// Keys dispatched once even while held: Backspace, Tab.
const ONCE: [i64; 2] = [37, 38];

/// One of the queue's events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueuedInput {
	/// A key's press (again each frame while held) or release.
	Key(KeyInput),
	/// What a key typed, with the `KeyModifier` mask held (for the UI only).
	Text(char, i64),
}

#[derive(Default)]
pub struct KeyQueue(RefCell<Vec<QueuedInput>>);

impl KeyQueue {
	/// A key went down (`down`) or up; the system's repeats can come in too.
	pub fn push(&self, key: i64, down: bool, modifier: i64) {
		let mut queue = self.0.borrow_mut();
		let held = queue
			.iter()
			.position(|e| matches!(e, QueuedInput::Key(e) if e.key == key && e.state == 1));
		match (held, down) {
			// Already held: a repeat.
			(Some(_), true) => return,
			// Released: it stops repeating.
			(Some(at), false) => {
				queue.remove(at);
			}
			(None, _) => {}
		}
		let state = i64::from(down);
		queue.push(QueuedInput::Key(KeyInput { key, state, modifier, fire: state }));
	}

	/// A key typed `c` (each of the system's repeats again).
	pub fn push_text(&self, c: char, modifier: i64) {
		self.0.borrow_mut().push(QueuedInput::Text(c, modifier));
	}

	/// This frame's events, in order, keeping the held keys for the next frame.
	pub fn take_frame(&self) -> Vec<QueuedInput> {
		let mut queue = self.0.borrow_mut();
		let frame = queue.clone();
		queue.retain(|e| matches!(e, QueuedInput::Key(e) if e.state == 1 && e.fire == 1 && !ONCE.contains(&e.key)));
		frame
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn keys(events: &[QueuedInput]) -> Vec<(i64, i64)> {
		events
			.iter()
			.map(|e| match e {
				QueuedInput::Key(e) => (e.key, e.state),
				QueuedInput::Text(c, _) => (-i64::from(u32::from(*c)), 0),
			})
			.collect()
	}

	#[test]
	fn what_a_key_types_comes_once_after_it() {
		let q = KeyQueue::default();
		q.push(13, true, 0);
		q.push_text('c', 0);
		assert_eq!(keys(&q.take_frame()), [(13, 1), (-99, 0)]);
		assert_eq!(keys(&q.take_frame()), [(13, 1)]);
	}

	#[test]
	fn a_held_key_comes_every_frame_until_released() {
		let q = KeyQueue::default();
		q.push(33, true, 0);
		assert_eq!(keys(&q.take_frame()), [(33, 1)]);
		assert_eq!(keys(&q.take_frame()), [(33, 1)], "again the next frame");
		q.push(33, true, 0);
		assert_eq!(keys(&q.take_frame()), [(33, 1)], "a system repeat adds nothing");
		q.push(33, false, 0);
		assert_eq!(keys(&q.take_frame()), [(33, 0)], "the release, once");
		assert!(q.take_frame().is_empty());
	}

	#[test]
	fn backspace_and_tab_come_once() {
		let q = KeyQueue::default();
		q.push(37, true, 0);
		q.push(14, true, 0);
		assert_eq!(keys(&q.take_frame()), [(37, 1), (14, 1)]);
		assert_eq!(keys(&q.take_frame()), [(14, 1)]);
	}
}
