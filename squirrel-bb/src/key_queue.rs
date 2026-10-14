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

use std::cell::RefCell;

use crate::input_events::KeyInput;

/// Keys dispatched once even while held: Backspace, Tab.
const ONCE: [i64; 2] = [37, 38];

#[derive(Default)]
pub struct KeyQueue(RefCell<Vec<KeyInput>>);

impl KeyQueue {
	/// A key went down (`down`) or up; the system's repeats can come in too.
	pub fn push(&self, key: i64, down: bool, modifier: i64) {
		let mut queue = self.0.borrow_mut();
		let held = queue.iter().position(|e| e.key == key && e.state == 1);
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
		queue.push(KeyInput { key, state, modifier, fire: state });
	}

	/// Every held key comes up (the window lost the keyboard).
	pub fn release_all(&self) {
		let held: Vec<(i64, i64)> = self.0.borrow().iter().filter(|e| e.state == 1).map(|e| (e.key, e.modifier)).collect();
		for (key, modifier) in held {
			self.push(key, false, modifier);
		}
	}

	/// This frame's events, in order, keeping the held keys for the next frame.
	pub fn take_frame(&self) -> Vec<KeyInput> {
		let mut queue = self.0.borrow_mut();
		let frame = queue.clone();
		queue.retain(|e| e.state == 1 && e.fire == 1 && !ONCE.contains(&e.key));
		frame
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn keys(events: &[KeyInput]) -> Vec<(i64, i64)> {
		events.iter().map(|e| (e.key, e.state)).collect()
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

	#[test]
	fn losing_the_keyboard_releases_held_keys() {
		let q = KeyQueue::default();
		q.push(11, true, 0);
		q.take_frame();
		q.release_all();
		assert_eq!(keys(&q.take_frame()), [(11, 0)]);
		assert!(q.take_frame().is_empty());
	}
}
