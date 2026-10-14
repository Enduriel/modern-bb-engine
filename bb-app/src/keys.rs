//! The keys as the scripts see them (`onKeyInput`'s `getKey()`): the engine's own
//! codes, not the platform's. The table is the one modders rely on (MSU's
//! `KeyMapSQ`), and it agrees with every code vanilla tests for (Escape 41, `C` 13,
//! `I` 19, F5 75, F9 79). Codes 42, 43, 52, 65 and 69 exist but nothing names them.
//!
//! A key reports state 1 when pressed and 0 when released (MSU's `getKeyState`);
//! vanilla acts on 0. The modifier is a mask whose bit 2 is Ctrl (vanilla's
//! developer keys test `getModifier() & 2`); the other bits are unknown.

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
	if state.control_key() { 2 } else { 0 }
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
}
