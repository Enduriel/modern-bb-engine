//! A player's live input from a window, as Coherent received it from the game: mouse
//! buttons held while the mouse moves, keys down and up as separate events (by their
//! Windows virtual-key code, see [`super::input`]), and the characters they type.

use cef::*;

use squirrel_bb::enums::{Key, KeyModifier};

use super::input::send_key;

/// Modifier keys held during an event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
	pub shift: bool,
	pub control: bool,
	pub alt: bool,
	/// The Mac's Command key (`metaKey`): only `bb` on macOS sends it.
	pub command: bool,
}

impl Modifiers {
	/// CEF's `EVENTFLAG_*` bits.
	pub(super) fn flags(self) -> u32 {
		u32::from(self.shift) << 1 | u32::from(self.control) << 2 | u32::from(self.alt) << 3 | u32::from(self.command) << 7
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

/// The Windows virtual-key code Coherent got for the engine's `key`: the original's UI
/// looks it up in its own table (built at `0x56cbe0`, read by `0x56d8c0`; disassembled
/// 2026-10-20), and a key not in it, the function keys, is -1 (probed 2026-10-21: F1
/// to F12 come to the page as keyCode -1).
pub fn coherent_key_code(key: Key) -> i32 {
	use Key::*;
	// The position of `key` in the run `first` to `last`, if it's in it.
	let run = |first: Key, last: Key| {
		(first.code()..=last.code()).contains(&key.code()).then(|| (key.code() - first.code()) as i32)
	};
	if let Some(i) = run(One, Nine) {
		return 0x31 + i;
	}
	if let Some(i) = run(A, Z) {
		return 0x41 + i;
	}
	if let Some(i) = run(Numpad0, Numpad9) {
		return 0x60 + i;
	}
	match key {
		Zero => 0x30,
		Backspace => 0x08,
		Tab => 0x09,
		Return => 0x0D,
		Space => 0x20,
		Escape => 0x1B,
		Pause => 0x13,
		Capital => 0x14,
		End => 0x23,
		Home => 0x24,
		PageUp => 0x21,
		PageDown => 0x22,
		ArrowLeft => 0x25,
		ArrowUp => 0x26,
		ArrowRight => 0x27,
		ArrowDown => 0x28,
		Insert => 0x2D,
		Delete => 0x2E,
		Multiply => 0x6A,
		Add => 0x6B,
		Subtract => 0x6D,
		Decimal => 0x6E,
		Divide => 0x6F,
		Control => 0x11,
		Shift => 0x10,
		Alt => 0x12,
		_ => -1,
	}
}

/// The modifier keys held, Command included.
pub fn modifiers_of(held: KeyModifier) -> Modifiers {
	Modifiers {
		shift: held.contains(KeyModifier::SHIFT),
		control: held.contains(KeyModifier::CONTROL),
		alt: held.contains(KeyModifier::ALT),
		command: held.contains(KeyModifier::COMMAND),
	}
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
	fn coherents_key_codes() {
		assert_eq!(coherent_key_code(Key::B), 66);
		assert_eq!(coherent_key_code(Key::One), 0x31);
		assert_eq!(coherent_key_code(Key::Zero), 0x30);
		assert_eq!(coherent_key_code(Key::Numpad9), 0x69);
		assert_eq!(coherent_key_code(Key::Escape), 0x1B);
		assert_eq!(coherent_key_code(Key::Pause), 0x13);
		assert_eq!(coherent_key_code(Key::PageUp), 0x21);
		assert_eq!(coherent_key_code(Key::Multiply), 0x6A);
		assert_eq!(coherent_key_code(Key::Alt), 0x12);
		// The function keys aren't in its table.
		assert_eq!(coherent_key_code(Key::F5), -1);
		assert_eq!(modifiers_of(KeyModifier::CONTROL | KeyModifier::ALT), Modifiers { shift: false, control: true, alt: true, command: false });
		assert_eq!(modifiers_of(KeyModifier::COMMAND), Modifiers { command: true, ..Modifiers::default() });
	}

	#[test]
	fn modifiers_are_cef_event_flags() {
		assert_eq!(Modifiers::default().flags(), 0);
		let all = Modifiers {
			shift: true,
			control: true,
			alt: true,
			command: true,
		};
		assert_eq!(all.flags(), 0b1000_1110);
		assert_eq!(MouseButton::Right.held_flag(), 1 << 6);
	}
}
