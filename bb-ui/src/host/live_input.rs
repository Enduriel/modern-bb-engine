//! A player's live input from a window, as Coherent received it from the game: mouse
//! buttons held while the mouse moves, keys down and up as separate events (by their
//! Windows virtual-key code, see [`super::input`]), and the characters they type.

use cef::*;

use super::input::send_key;

/// Modifier keys held during an event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
	pub shift: bool,
	pub control: bool,
	pub alt: bool,
}

impl Modifiers {
	/// CEF's `EVENTFLAG_*` bits.
	pub(super) fn flags(self) -> u32 {
		u32::from(self.shift) << 1 | u32::from(self.control) << 2 | u32::from(self.alt) << 3
	}
}

/// A mouse button the page can see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
	Left,
	Middle,
	Right,
}

impl MouseButton {
	fn cef(self) -> MouseButtonType {
		match self {
			MouseButton::Left => MouseButtonType::LEFT,
			MouseButton::Middle => MouseButtonType::MIDDLE,
			MouseButton::Right => MouseButtonType::RIGHT,
		}
	}

	/// The `EVENTFLAG_*_MOUSE_BUTTON` bit a move carries while the button is held.
	pub(super) fn held_flag(self) -> u32 {
		match self {
			MouseButton::Left => 1 << 4,
			MouseButton::Middle => 1 << 5,
			MouseButton::Right => 1 << 6,
		}
	}
}

/// `button` went down or up at `(x, y)`, with `held` (`EVENTFLAG_*` bits) on the event,
/// as the `count`th click of a run (2 for a double click).
pub(super) fn button(
	host: &BrowserHost,
	x: i32,
	y: i32,
	button: MouseButton,
	down: bool,
	held: u32,
	count: i32,
) {
	let event = MouseEvent {
		x,
		y,
		modifiers: held,
	};
	host.send_mouse_move_event(Some(&event), 0);
	host.send_mouse_click_event(Some(&event), button.cef(), i32::from(!down), count);
}

/// Key `code` went down or up.
pub(super) fn key(host: &BrowserHost, code: i32, down: bool, modifiers: Modifiers) {
	// An offscreen browser only delivers keys while it has focus.
	host.set_focus(1);
	let type_ = if down {
		KeyEventType::RAWKEYDOWN
	} else {
		KeyEventType::KEYUP
	};
	send_key(host, type_, code, modifiers.flags(), 0);
}

/// The keys typed `c` (Windows' WM_CHAR, after the key went down).
pub(super) fn character(host: &BrowserHost, c: char, modifiers: Modifiers) {
	host.set_focus(1);
	let mut units = [0; 2];
	for &unit in c.encode_utf16(&mut units).iter() {
		// For character events, CEF takes the character as the key code too.
		send_key(
			host,
			KeyEventType::CHAR,
			i32::from(unit),
			modifiers.flags(),
			unit,
		);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn modifiers_are_cef_event_flags() {
		assert_eq!(Modifiers::default().flags(), 0);
		let all = Modifiers {
			shift: true,
			control: true,
			alt: true,
		};
		assert_eq!(all.flags(), 0b1110);
		assert_eq!(MouseButton::Right.held_flag(), 1 << 6);
	}
}
