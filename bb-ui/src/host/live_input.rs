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

/// The Windows virtual-key code Coherent got for the engine's key `code`: the
/// original's UI looks it up in its own table (built at `0x56cbe0`, read by `0x56d8c0`;
/// disassembled 2026-10-20), and a key not in it, the function keys, is -1 (probed
/// 2026-10-21: F1 to F12 come to the page as keyCode -1).
pub fn coherent_key_code(code: i64) -> i32 {
	match code {
		1..=9 => 0x30 + code as i32,
		10 => 0x30,
		11..=36 => 0x41 + (code - 11) as i32,
		37 => 0x08,
		38 => 0x09,
		39 => 0x0D,
		40 => 0x20,
		41 => 0x1B,
		42 => 0x13,
		43 => 0x14,
		44 => 0x23,
		45 => 0x24,
		46 => 0x21,
		47 => 0x22,
		48 => 0x25,
		49 => 0x26,
		50 => 0x27,
		51 => 0x28,
		53 => 0x2D,
		54 => 0x2E,
		55..=64 => 0x60 + (code - 55) as i32,
		66 => 0x6A,
		67 => 0x6B,
		68 => 0x6D,
		69 => 0x6E,
		70 => 0x6F,
		95 => 0x11,
		96 => 0x10,
		97 => 0x12,
		_ => -1,
	}
}

/// The modifier keys in the engine's `KeyModifier` mask (Shift 1, Ctrl 2, Alt 4), and
/// the host's Command bit ([`squirrel_bb::KEY_MODIFIER_COMMAND`]).
pub fn modifiers_of_mask(mask: i64) -> Modifiers {
	Modifiers {
		shift: mask & 1 != 0,
		control: mask & 2 != 0,
		alt: mask & 4 != 0,
		command: mask & squirrel_bb::KEY_MODIFIER_COMMAND != 0,
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
		assert_eq!(coherent_key_code(12), 66);
		assert_eq!(coherent_key_code(10), 0x30);
		assert_eq!(coherent_key_code(41), 0x1B);
		assert_eq!(coherent_key_code(42), 0x13);
		assert_eq!(coherent_key_code(46), 0x21);
		assert_eq!(coherent_key_code(66), 0x6A);
		assert_eq!(coherent_key_code(97), 0x12);
		// The function keys aren't in its table.
		assert_eq!(coherent_key_code(75), -1);
		assert_eq!(modifiers_of_mask(6), Modifiers { shift: false, control: true, alt: true, command: false });
		assert_eq!(modifiers_of_mask(8), Modifiers { command: true, ..Modifiers::default() });
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
