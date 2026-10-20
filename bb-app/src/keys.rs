//! The keys as the scripts see them (`onKeyInput`'s `getKey()`): the engine's own
//! codes, not the platform's. The original takes its keys from SFML 2 and turns
//! SFML's key into its code with a table (filled at `0x5b6a20`, read at `0x5b67d5`;
//! disassembled 2026-10-20). Letters are 11 to 36, digits 1 to 10, F1 to F15 71 to 85,
//! Pause 42, PageUp 46 and PageDown 47 (MSU's `KeyMapSQ` names those two the other way
//! round). A key SFML has no name for (CapsLock, the Windows and menu keys, the
//! punctuation keys, the numpad's decimal point, F16 and up) gets no code, and the
//! original drops its events: neither the states nor the page see them.
//!
//! SFML names a key by its Windows virtual-key code, and that follows the keyboard
//! layout: AZERTY's `A` (the key a US keyboard has `Q` on) is `A`. So the code comes
//! from what the layout makes of a key where that's a letter (see [`code_for`]).
//!
//! A key reports state 1 when pressed and 0 when released (`KeyState` Down/Up); vanilla
//! acts on 0. Held-down repeats come again as pressed with `getFire()` 1 (`KeyFire`
//! Repeated). The modifier is the `KeyModifier` mask: Shift 1, Ctrl 2, Alt 4 (the
//! original's enums, disassembled).
//!
//! The UI page gets the same keys, by the Windows virtual-key code the original's UI
//! gives each engine code ([`windows_code`]).

use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey};

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
	// SFML 2 names F1 to F15.
	let functions = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15];
	if let Some(i) = functions.iter().position(|&k| k == key) {
		return Some(71 + i as i64);
	}
	Some(match key {
		Backspace => 37,
		Tab => 38,
		Enter | NumpadEnter => 39,
		Space => 40,
		Escape => 41,
		Pause => 42,
		End => 44,
		Home => 45,
		PageUp => 46,
		PageDown => 47,
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

/// The engine's code for the key at `physical`, which the keyboard layout makes
/// `logical`, as the original's Windows virtual-key codes name it:
/// - A key the layout makes a letter is that letter, wherever it is (AZERTY's `A`
///   on the US `Q`, its `M` on the US `;`). A letter outside ASCII (a Cyrillic layout's)
///   leaves the key as it is: Windows names those layouts' keys as US ones.
/// - A numpad key with NumLock off is the key it stands for (Home, an arrow, Delete),
///   or none (the middle 5).
/// - Any other key (the digits, whatever they type) is the key itself.
pub fn code_for(physical: KeyCode, logical: &Key) -> Option<i64> {
	code(layout_key(physical, logical)?)
}

/// The key the layout makes of `physical` (see [`code_for`]).
fn layout_key(physical: KeyCode, logical: &Key) -> Option<KeyCode> {
	use KeyCode::*;
	match logical {
		Key::Character(text) => {
			let mut chars = text.chars();
			match (chars.next(), chars.next()) {
				(Some(c), None) if c.is_ascii_alphabetic() => {
					let letter = c.to_ascii_uppercase() as u8 - b'A';
					let letters = [
						KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP,
						KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
					];
					Some(letters[usize::from(letter)])
				}
				_ => Some(physical),
			}
		}
		Key::Named(named) if is_numpad_digit(physical) => match named {
			NamedKey::Home => Some(Home),
			NamedKey::End => Some(End),
			NamedKey::PageUp => Some(PageUp),
			NamedKey::PageDown => Some(PageDown),
			NamedKey::ArrowLeft => Some(ArrowLeft),
			NamedKey::ArrowUp => Some(ArrowUp),
			NamedKey::ArrowRight => Some(ArrowRight),
			NamedKey::ArrowDown => Some(ArrowDown),
			NamedKey::Insert => Some(Insert),
			NamedKey::Delete => Some(Delete),
			_ => None,
		},
		_ => Some(physical),
	}
}

/// Whether `key` is one of the numpad's keys that NumLock turns into others.
fn is_numpad_digit(key: KeyCode) -> bool {
	use KeyCode::*;
	matches!(
		key,
		Numpad0 | Numpad1 | Numpad2 | Numpad3 | Numpad4 | Numpad5 | Numpad6 | Numpad7 | Numpad8 | Numpad9 | NumpadDecimal
	)
}

/// Whether `key` pressed with `modifiers` held shows or hides the FPS display:
/// Ctrl+Shift+F. Neither vanilla nor the mods surveyed bind it (vanilla's keys are
/// plain letters, digits and F2 to F11; the mods' MSU keybinds take F1 to F11 and a few
/// other Ctrl+Shift letters).
pub fn toggles_fps(key: KeyCode, modifiers: ModifiersState) -> bool {
	key == KeyCode::KeyF && modifiers == ModifiersState::CONTROL | ModifiersState::SHIFT
}

/// The modifier mask for `getModifier()`.
pub fn modifier(state: ModifiersState) -> i64 {
	i64::from(state.shift_key()) | i64::from(state.control_key()) << 1 | i64::from(state.alt_key()) << 2
}

/// The Windows virtual-key code the page gets for the engine's `code` (its `keyCode`),
/// if any: the original's UI looks it up in its own table (built at `0x56cbe0`, read by
/// `0x56d8c0`; disassembled 2026-10-20). The function keys aren't in it (Coherent gets
/// -1 for them), so the page sees no key event for them here, nor for the keys the
/// engine has no code for; what they type still reaches it.
pub fn windows_code(code: i64) -> Option<i32> {
	Some(match code {
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
	}

	#[test]
	fn codes_from_the_originals_table() {
		// world_state pauses on 42 (with Space and 0).
		assert_eq!(code(KeyCode::Pause), Some(42));
		assert_eq!(code(KeyCode::PageUp), Some(46));
		assert_eq!(code(KeyCode::PageDown), Some(47));
		assert_eq!(code(KeyCode::F15), Some(85));
		assert_eq!(code(KeyCode::F16), None);
		assert_eq!(code(KeyCode::CapsLock), None);
		assert_eq!(code(KeyCode::Comma), None);
		assert_eq!(code(KeyCode::NumpadDecimal), None);
		assert_eq!(code(KeyCode::SuperLeft), None);
	}

	#[test]
	fn letters_follow_the_layout() {
		let typed = |s: &str| Key::Character(s.into());
		// AZERTY: the US `Q` key types `a`, the US `;` key `m`.
		assert_eq!(code_for(KeyCode::KeyQ, &typed("a")), Some(11));
		assert_eq!(code_for(KeyCode::Semicolon, &typed("m")), Some(23));
		// With Shift.
		assert_eq!(code_for(KeyCode::KeyW, &typed("Z")), Some(36));
		// AZERTY's digit row types `&` unshifted: still the digit.
		assert_eq!(code_for(KeyCode::Digit1, &typed("&")), Some(1));
		// A Cyrillic letter: the key as a US keyboard names it.
		assert_eq!(code_for(KeyCode::KeyA, &typed("ф")), Some(11));
		// AltGr+E on a German layout types `€`.
		assert_eq!(code_for(KeyCode::KeyE, &typed("€")), Some(15));
		assert_eq!(code_for(KeyCode::Escape, &Key::Named(NamedKey::Escape)), Some(41));
	}

	#[test]
	fn numpad_follows_num_lock() {
		assert_eq!(code_for(KeyCode::Numpad7, &Key::Character("7".into())), Some(62));
		assert_eq!(code_for(KeyCode::Numpad7, &Key::Named(NamedKey::Home)), Some(45));
		assert_eq!(code_for(KeyCode::Numpad8, &Key::Named(NamedKey::ArrowUp)), Some(49));
		assert_eq!(code_for(KeyCode::NumpadDecimal, &Key::Named(NamedKey::Delete)), Some(54));
		assert_eq!(code_for(KeyCode::Numpad5, &Key::Named(NamedKey::Clear)), None);
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
		assert_eq!(windows_code(12), Some(66));
		assert_eq!(windows_code(10), Some(0x30));
		assert_eq!(windows_code(41), Some(0x1B));
		assert_eq!(windows_code(39), Some(0x0D));
		assert_eq!(windows_code(46), Some(0x21));
		assert_eq!(windows_code(66), Some(0x6A));
		assert_eq!(windows_code(97), Some(0x12));
		// The function keys have none.
		assert_eq!(windows_code(75), None);
		// Every code the keys make has its own (but the function keys).
		let mut seen = std::collections::HashSet::new();
		for engine in (1..=70).chain(95..=97) {
			if let Some(vk) = windows_code(engine) {
				assert!(seen.insert(vk), "{engine} shares {vk:#x}");
			}
		}
	}
}
