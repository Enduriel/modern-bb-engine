//! The keys as the scripts see them (`onKeyInput`'s `getKey()`): the engine's own
//! codes, not the platform's. The table is the one modders rely on (MSU's
//! `KeyMapSQ`), and it agrees with every code vanilla tests for (Escape 41, `C` 13,
//! `I` 19, F5 75, F9 79). Codes 42, 43, 52, 65 and 69 exist but nothing names them.
//!
//! A key reports state 1 when pressed and 0 when released (`KeyState` Down/Up); vanilla
//! acts on 0. Held-down repeats come again as pressed with `getFire()` 1 (`KeyFire`
//! Repeated). The modifier is the `KeyModifier` mask: Shift 1, Ctrl 2, Alt 4 (the
//! original's enums, disassembled).
//!
//! The UI page gets keys as Coherent did, by their Windows virtual-key code
//! ([`windows_code`]).

use winit::keyboard::{KeyCode, ModifiersState};

/// The engine's code for `key`, if the scripts can see it.
pub fn code(key: KeyCode) -> Option<i64> {
	use KeyCode::*;
	let letters = [
		KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR,
		KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
	];
	if let Some(i) = letters.iter().position(|&k| k == key) {
		return Some(11 + i as i64);
	}
	let digits = [Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9, Digit0];
	if let Some(i) = digits.iter().position(|&k| k == key) {
		return Some(1 + i as i64);
	}
	let numpad = [
		Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
	];
	if let Some(i) = numpad.iter().position(|&k| k == key) {
		return Some(55 + i as i64);
	}
	let functions = [
		F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24,
	];
	if let Some(i) = functions.iter().position(|&k| k == key) {
		return Some(71 + i as i64);
	}
	Some(match key {
		Backspace => 37,
		Tab => 38,
		Enter | NumpadEnter => 39,
		Space => 40,
		Escape => 41,
		End => 44,
		Home => 45,
		PageDown => 46,
		PageUp => 47,
		ArrowLeft => 48,
		ArrowUp => 49,
		ArrowRight => 50,
		ArrowDown => 51,
		Insert => 53,
		Delete => 54,
		NumpadMultiply => 66,
		NumpadAdd => 67,
		NumpadSubtract => 68,
		NumpadDivide => 70,
		ControlLeft | ControlRight => 95,
		ShiftLeft | ShiftRight => 96,
		AltLeft | AltRight => 97,
		_ => return None,
	})
}

/// The modifier mask for `getModifier()`.
pub fn modifier(state: ModifiersState) -> i64 {
	i64::from(state.shift_key()) | i64::from(state.control_key()) << 1 | i64::from(state.alt_key()) << 2
}

/// The Windows virtual-key code of `key` (what pages see as `keyCode`), if it has one.
pub fn windows_code(key: KeyCode) -> Option<i32> {
	use KeyCode::*;
	let letters = [
		KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR,
		KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
	];
	if let Some(i) = letters.iter().position(|&k| k == key) {
		return Some(0x41 + i as i32);
	}
	let digits = [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9];
	if let Some(i) = digits.iter().position(|&k| k == key) {
		return Some(0x30 + i as i32);
	}
	let numpad = [
		Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
	];
	if let Some(i) = numpad.iter().position(|&k| k == key) {
		return Some(0x60 + i as i32);
	}
	let functions = [
		F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24,
	];
	if let Some(i) = functions.iter().position(|&k| k == key) {
		return Some(0x70 + i as i32);
	}
	Some(match key {
		Backspace => 0x08,
		Tab => 0x09,
		Enter | NumpadEnter => 0x0D,
		ShiftLeft | ShiftRight => 0x10,
		ControlLeft | ControlRight => 0x11,
		AltLeft | AltRight => 0x12,
		Pause => 0x13,
		CapsLock => 0x14,
		Escape => 0x1B,
		Space => 0x20,
		PageUp => 0x21,
		PageDown => 0x22,
		End => 0x23,
		Home => 0x24,
		ArrowLeft => 0x25,
		ArrowUp => 0x26,
		ArrowRight => 0x27,
		ArrowDown => 0x28,
		Insert => 0x2D,
		Delete => 0x2E,
		NumpadMultiply => 0x6A,
		NumpadAdd => 0x6B,
		NumpadSubtract => 0x6D,
		NumpadDecimal => 0x6E,
		NumpadDivide => 0x6F,
		Semicolon => 0xBA,
		Equal => 0xBB,
		Comma => 0xBC,
		Minus => 0xBD,
		Period => 0xBE,
		Slash => 0xBF,
		Backquote => 0xC0,
		BracketLeft => 0xDB,
		Backslash => 0xDC,
		BracketRight => 0xDD,
		Quote => 0xDE,
		_ => return None,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn codes_vanilla_tests_for() {
		assert_eq!(code(KeyCode::Escape), Some(41));
		assert_eq!(code(KeyCode::KeyC), Some(13));
		assert_eq!(code(KeyCode::KeyI), Some(19));
		assert_eq!(code(KeyCode::Space), Some(40));
		assert_eq!(code(KeyCode::Digit1), Some(1));
		assert_eq!(code(KeyCode::Digit0), Some(10));
		assert_eq!(code(KeyCode::F5), Some(75));
		assert_eq!(code(KeyCode::F9), Some(79));
		assert_eq!(code(KeyCode::KeyZ), Some(36));
		assert_eq!(code(KeyCode::CapsLock), None);
	}

	#[test]
	fn modifiers_are_a_mask() {
		assert_eq!(modifier(ModifiersState::empty()), 0);
		assert_eq!(modifier(ModifiersState::SHIFT), 1);
		assert_eq!(modifier(ModifiersState::CONTROL), 2);
		assert_eq!(modifier(ModifiersState::ALT | ModifiersState::SHIFT), 5);
	}

	#[test]
	fn windows_codes_for_the_page() {
		// What vanilla's text fields accept (`ui/controls/input.js`): `0`-`Z` by code.
		assert_eq!(windows_code(KeyCode::KeyB), Some(66));
		assert_eq!(windows_code(KeyCode::Digit0), Some(0x30));
		assert_eq!(windows_code(KeyCode::Escape), Some(0x1B));
		assert_eq!(windows_code(KeyCode::Enter), Some(0x0D));
		assert_eq!(windows_code(KeyCode::F5), Some(0x74));
		assert_eq!(windows_code(KeyCode::Comma), Some(0xBC));
	}
}
