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
//! Nothing comes up when the window loses the keyboard, but the queue forgets its keys:
//! the original's main loop (`0x5b3e4c`), seeing the window's focus change (its
//! `hasFocus`, `0x5b4880`, the flag `0x5b57bd` notes), clears the queue's pending and
//! held events without dispatching them (`0x551460`) and drops the input that comes
//! while the window lacks the keyboard (the queue's flag `+0x92`, `0x5517f0`); it
//! clears them again as the window gets the keyboard back. So the states (and the page)
//! never see the held key's release, and its presses stop at once (probed 2026-10-21: a
//! held key's keydowns to the page and presses to the states stopped as the keyboard
//! went, with no release; with the keyboard back and the key still down, it came again
//! with the system's next repeat, as a new press).
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
pub struct KeyQueue {
	queue: RefCell<Vec<QueuedInput>>,
	/// The window lacks the keyboard: input is dropped.
	unfocused: std::cell::Cell<bool>,
}

impl KeyQueue {
	/// The window got the keyboard, or lost it: the queue forgets what it holds, and
	/// while the window lacks it, takes nothing in (see the module notes).
	pub fn set_focused(&self, focused: bool) {
		if self.unfocused.replace(!focused) == focused {
			self.queue.borrow_mut().clear();
		}
	}

	/// A key went down (`down`) or up; the system's repeats can come in too.
	pub fn push(&self, key: i64, down: bool, modifier: i64) {
		if self.unfocused.get() {
			return;
		}
		let mut queue = self.queue.borrow_mut();
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
		if !self.unfocused.get() {
			self.queue.borrow_mut().push(QueuedInput::Text(c, modifier));
		}
	}

	/// This frame's events, in order, keeping the held keys for the next frame.
	pub fn take_frame(&self) -> Vec<QueuedInput> {
		let mut queue = self.queue.borrow_mut();
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

	/// Alt+Tab: Alt's press is held as the window loses the keyboard, its release comes
	/// elsewhere. It used to stay held, every frame, for good (the page never rested).
	#[test]
	fn losing_the_keyboard_forgets_the_held_keys_without_a_release() {
		let q = KeyQueue::default();
		q.push(97, true, 4);
		q.push(33, true, 0);
		assert_eq!(keys(&q.take_frame()), [(97, 1), (33, 1)]);
		q.set_focused(false);
		assert!(q.take_frame().is_empty(), "no release, and no more presses");
		q.push(97, false, 0);
		q.push_text('w', 0);
		assert!(q.take_frame().is_empty(), "input while the window lacks the keyboard is dropped");
		q.set_focused(false);
		q.set_focused(true);
		assert!(q.take_frame().is_empty());
		// W still down: the system's next repeat is a new press.
		q.push(33, true, 0);
		assert_eq!(keys(&q.take_frame()), [(33, 1)]);
		assert_eq!(keys(&q.take_frame()), [(33, 1)]);
		// Getting the keyboard back clears it too.
		q.set_focused(true);
		assert_eq!(keys(&q.take_frame()), [(33, 1)], "no change, nothing cleared");
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
