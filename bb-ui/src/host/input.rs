//! Keyboard and wheel input, in the form Coherent received it from the game: keys by
//! their Windows virtual-key code (what pages see as `keyCode`/`which`, and what
//! `ui/keys.js` names), and 120 per wheel notch (`wheelDelta`, which
//! jquery.mousewheel divides by 120).
//!
//! Exact codes matter: vanilla's text fields (`ui/controls/input.js`) refuse any key
//! whose code is outside `0`–`Z` apart from a few named ones, so a typed `b` has to
//! arrive with code 66, as on Windows, not its character code 98.

use cef::*;

/// `EVENTFLAG_SHIFT_DOWN` (`cef_types.h`).
const SHIFT_DOWN: u32 = 1 << 1;

/// A key that is pressed rather than typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
	Backspace,
	Tab,
	Enter,
	Escape,
	Space,
	PageUp,
	PageDown,
	End,
	Home,
	Left,
	Up,
	Right,
	Down,
	Delete,
}

impl Key {
	/// Every key, with the name [`Key::from_name`] accepts.
	pub const ALL: [(Key, &'static str); 14] = [
		(Key::Backspace, "Backspace"),
		(Key::Tab, "Tab"),
		(Key::Enter, "Enter"),
		(Key::Escape, "Escape"),
		(Key::Space, "Space"),
		(Key::PageUp, "PageUp"),
		(Key::PageDown, "PageDown"),
		(Key::End, "End"),
		(Key::Home, "Home"),
		(Key::Left, "Left"),
		(Key::Up, "Up"),
		(Key::Right, "Right"),
		(Key::Down, "Down"),
		(Key::Delete, "Delete"),
	];

	/// The key called `name` (as in [`Key::ALL`], any case).
	pub fn from_name(name: &str) -> Option<Key> {
		Key::ALL
			.iter()
			.find(|(_, n)| n.eq_ignore_ascii_case(name))
			.map(|&(key, _)| key)
	}

	/// The Windows virtual-key code.
	pub fn code(self) -> i32 {
		match self {
			Key::Backspace => 0x08,
			Key::Tab => 0x09,
			Key::Enter => 0x0D,
			Key::Escape => 0x1B,
			Key::Space => 0x20,
			Key::PageUp => 0x21,
			Key::PageDown => 0x22,
			Key::End => 0x23,
			Key::Home => 0x24,
			Key::Left => 0x25,
			Key::Up => 0x26,
			Key::Right => 0x27,
			Key::Down => 0x28,
			Key::Delete => 0x2E,
		}
	}

	/// The character a press also sends (Windows' WM_CHAR), if any.
	fn character(self) -> Option<char> {
		match self {
			Key::Enter => Some('\r'),
			Key::Space => Some(' '),
			_ => None,
		}
	}
}

/// The US-layout keys for punctuation: the unshifted and shifted characters, and the
/// key's virtual-key code (`VK_OEM_*`).
const PUNCTUATION: [(char, char, i32); 11] = [
	(';', ':', 0xBA),
	('=', '+', 0xBB),
	(',', '<', 0xBC),
	('-', '_', 0xBD),
	('.', '>', 0xBE),
	('/', '?', 0xBF),
	('`', '~', 0xC0),
	('[', '{', 0xDB),
	('\\', '|', 0xDC),
	(']', '}', 0xDD),
	('\'', '"', 0xDE),
];

/// The digit keys' shifted characters, from `1` to `0`.
const SHIFTED_DIGITS: &str = "!@#$%^&*()";

/// The virtual-key code and modifiers that type `c` on a US layout. Characters the
/// layout has no key for (other scripts) get code 0. Vanilla's text fields refuse
/// codes above `Z`, so punctuation never reached them in the original either.
fn key_for(c: char) -> (i32, u32) {
	match c {
		'a'..='z' => (c.to_ascii_uppercase() as i32, 0),
		'A'..='Z' => (c as i32, SHIFT_DOWN),
		'0'..='9' | ' ' => (c as i32, 0),
		_ => {
			if let Some(i) = SHIFTED_DIGITS.find(c) {
				return (b"1234567890"[i] as i32, SHIFT_DOWN);
			}
			for (plain, shifted, code) in PUNCTUATION {
				if c == plain {
					return (code, 0);
				}
				if c == shifted {
					return (code, SHIFT_DOWN);
				}
			}
			(0, 0)
		}
	}
}

pub(super) fn send_key(
	host: &BrowserHost,
	type_: KeyEventType,
	code: i32,
	modifiers: u32,
	character: u16,
) {
	host.send_key_event(Some(&KeyEvent {
		type_,
		modifiers,
		windows_key_code: code,
		character,
		unmodified_character: character,
		..Default::default()
	}));
}

/// One press: key down, its character if it has one, key up.
fn press(host: &BrowserHost, code: i32, modifiers: u32, character: Option<char>) {
	// An offscreen browser only delivers keys while it has focus.
	host.set_focus(1);
	send_key(host, KeyEventType::RAWKEYDOWN, code, modifiers, 0);
	if let Some(c) = character {
		let mut units = [0; 2];
		for &unit in c.encode_utf16(&mut units).iter() {
			// For character events, CEF takes the character as the key code too.
			send_key(host, KeyEventType::CHAR, i32::from(unit), modifiers, unit);
		}
	}
	send_key(host, KeyEventType::KEYUP, code, modifiers, 0);
}

/// Press and release `key` in the focused element.
pub(super) fn press_key(host: &BrowserHost, key: Key) {
	press(host, key.code(), 0, key.character());
}

/// Type `text` into the focused element, one key press per character.
pub(super) fn type_text(host: &BrowserHost, text: &str) {
	for c in text.chars() {
		let (code, modifiers) = key_for(c);
		press(host, code, modifiers, Some(c));
	}
}

/// What the original's page gets per notch of the wheel, as a wheel delta: its UI hands
/// Coherent a `WheelY` of ±30 per notch (`0x56cae9`, `0x56caf8`; disassembled
/// 2026-10-14), which Coherent's WebKit counts as 30 ticks of 120 (`0x10603c4d`), so
/// `wheelDelta` is ±3600. Vanilla's lists scroll by `wheelDelta / 120` (jquery.mousewheel)
/// times twice their own delta (aciScrollBar), so a notch scrolls 30 times what a 120
/// would. Chromium counts the same ticks from this delta.
pub const WHEEL_DELTA_PER_NOTCH: i32 = 30 * 120;

/// Turn the wheel `notches` times over (x, y): positive is away from the player
/// (scrolling up), as `wheelDelta` counts.
pub(super) fn wheel(host: &BrowserHost, x: i32, y: i32, notches: i32) {
	let event = MouseEvent { x, y, modifiers: 0 };
	host.send_mouse_move_event(Some(&event), 0);
	host.send_mouse_wheel_event(Some(&event), 0, WHEEL_DELTA_PER_NOTCH * notches);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn letters_use_their_windows_key_codes() {
		assert_eq!(key_for('b'), (66, 0));
		assert_eq!(key_for('B'), (66, SHIFT_DOWN));
		assert_eq!(key_for('7'), (55, 0));
		assert_eq!(key_for(' '), (32, 0));
		assert_eq!(key_for('-'), (0xBD, 0));
		assert_eq!(key_for('_'), (0xBD, SHIFT_DOWN));
		assert_eq!(key_for('!'), ('1' as i32, SHIFT_DOWN));
		assert_eq!(key_for(')'), ('0' as i32, SHIFT_DOWN));
		assert_eq!(key_for('"'), (0xDE, SHIFT_DOWN));
		assert_eq!(key_for('é'), (0, 0));
	}

	#[test]
	fn keys_by_name() {
		assert_eq!(Key::from_name("enter"), Some(Key::Enter));
		assert_eq!(Key::from_name("Backspace").map(Key::code), Some(8));
		assert_eq!(Key::from_name("Delete").map(Key::code), Some(46));
		assert_eq!(Key::from_name("F13"), None);
	}
}
