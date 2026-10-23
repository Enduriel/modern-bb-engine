//! Presenting the last frame again while the mouse moves.
//!
//! A frame that would show just what the last did is neither drawn nor presented (see
//! [`crate::shown`]). With the system's cursor (the default, `game_cursor`) a mouse move
//! changes nothing in the frame, so a still menu presented a few frames a second as the
//! mouse moved over it. On a variable-refresh display (G-Sync, FreeSync) the monitor
//! refreshes only when a frame is presented, and the hardware cursor moves only with
//! the refresh: a player on a 360 Hz monitor saw it move at ~20 Hz, choppy and
//! ghosting. The original presents every refresh.
//!
//! So a frame that would show what the last did is presented anyway, the last frame
//! again (the kept frame blitted, as every present does), when the mouse moved since the
//! last frame presented and the system shows the cursor over the window. A still mouse
//! still costs no present. It applies on every system: where the compositor refreshes
//! anyway, a present more while the mouse moves is harmless.
//!
//! Left out: a cursor the game draws (`CursorShow::Drawn`), whose move changes the
//! frame and is drawn anyway; no cursor shown (`Hidden`); and a window the system says
//! isn't shown (occluded, minimized), which a present wouldn't reach.

use crate::game_cursor::CursorShow;

/// Whether the mouse moved since the last frame presented.
#[derive(Debug, Default)]
pub struct MouseRefresh {
	moved: bool,
}

impl MouseRefresh {
	/// The mouse moved over the window.
	pub fn moved(&mut self) {
		self.moved = true;
	}

	/// A frame was presented: the moves so far show.
	pub fn presented(&mut self) {
		self.moved = false;
	}

	/// For a frame that would show what the last did: whether to present the last frame
	/// again, with `cursor` shown over the window and the window `occluded` or not. The
	/// moves are taken either way: a cursor the game draws or none shown needs no present
	/// for them, and a window not shown is presented again when it shows.
	pub fn take(&mut self, cursor: Option<&CursorShow>, occluded: bool) -> bool {
		let moved = std::mem::take(&mut self.moved);
		moved && !occluded && matches!(cursor, Some(CursorShow::File(_) | CursorShow::Sprite { .. }))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sprite() -> CursorShow {
		CursorShow::Sprite { brush: "cursor_hand".into(), frame: 0 }
	}

	#[test]
	fn a_moving_mouse_presents_every_frame_and_a_still_one_none() {
		let mut refresh = MouseRefresh::default();
		let cursor = sprite();
		// Still: nothing to present, frame after frame.
		for _ in 0..10 {
			assert!(!refresh.take(Some(&cursor), false));
		}
		// Moving: a move (or several) before each frame presents it.
		for moves in 1..=10 {
			for _ in 0..moves {
				refresh.moved();
			}
			assert!(refresh.take(Some(&cursor), false), "after {moves} moves");
		}
		// Stopped: the frame after the last move presented, then none.
		assert!(!refresh.take(Some(&cursor), false));
		// The original's hardware cursor shows over the window as the sprite does.
		refresh.moved();
		assert!(refresh.take(Some(&CursorShow::File("cursor_hand".into())), false));
	}

	#[test]
	fn a_frame_drawn_and_presented_shows_the_moves() {
		let mut refresh = MouseRefresh::default();
		refresh.moved();
		refresh.presented();
		assert!(!refresh.take(Some(&sprite()), false), "presented already");
	}

	#[test]
	fn no_present_for_a_drawn_cursor_none_shown_or_a_window_not_shown() {
		let mut refresh = MouseRefresh::default();
		for (cursor, occluded) in [(Some(CursorShow::Drawn), false), (Some(CursorShow::Hidden), false), (None, false), (Some(sprite()), true)] {
			refresh.moved();
			assert!(!refresh.take(cursor.as_ref(), occluded), "{cursor:?}, occluded {occluded}");
			// The moves were taken: nothing left over for a later frame.
			assert!(!refresh.take(Some(&sprite()), false));
		}
	}
}
