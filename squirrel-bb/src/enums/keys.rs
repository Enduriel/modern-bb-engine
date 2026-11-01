//! `Key` (`getKey()`), `KeyState` (`getState()`) and `KeyFire` (`getFire()`).

use super::script_enum;

script_enum! {
	/// The engine's key codes, not the platform's (see `bb-app`'s `keys` for how a
	/// platform key becomes one). Some never come from the original's SFML key table
	/// (F16 to F24, `Capital`, `Scroll`, `Decimal`).
	Key {
		One = 1,
		Two = 2,
		Three = 3,
		Four = 4,
		Five = 5,
		Six = 6,
		Seven = 7,
		Eight = 8,
		Nine = 9,
		Zero = 10,
		A = 11,
		B = 12,
		C = 13,
		D = 14,
		E = 15,
		F = 16,
		G = 17,
		H = 18,
		I = 19,
		J = 20,
		K = 21,
		L = 22,
		M = 23,
		N = 24,
		O = 25,
		P = 26,
		Q = 27,
		R = 28,
		S = 29,
		T = 30,
		U = 31,
		V = 32,
		W = 33,
		X = 34,
		Y = 35,
		Z = 36,
		Backspace = 37,
		Tab = 38,
		Return = 39,
		Space = 40,
		Escape = 41,
		Pause = 42,
		Capital = 43,
		End = 44,
		Home = 45,
		PageUp = 46,
		PageDown = 47,
		ArrowLeft = 48,
		ArrowUp = 49,
		ArrowRight = 50,
		ArrowDown = 51,
		Scroll = 52,
		Insert = 53,
		Delete = 54,
		Numpad0 = 55,
		Numpad1 = 56,
		Numpad2 = 57,
		Numpad3 = 58,
		Numpad4 = 59,
		Numpad5 = 60,
		Numpad6 = 61,
		Numpad7 = 62,
		Numpad8 = 63,
		Numpad9 = 64,
		NumpadEnter = 65,
		Multiply = 66,
		Add = 67,
		Subtract = 68,
		Decimal = 69,
		Divide = 70,
		F1 = 71,
		F2 = 72,
		F3 = 73,
		F4 = 74,
		F5 = 75,
		F6 = 76,
		F7 = 77,
		F8 = 78,
		F9 = 79,
		F10 = 80,
		F11 = 81,
		F12 = 82,
		F13 = 83,
		F14 = 84,
		F15 = 85,
		F16 = 86,
		F17 = 87,
		F18 = 88,
		F19 = 89,
		F20 = 90,
		F21 = 91,
		F22 = 92,
		F23 = 93,
		F24 = 94,
		Control = 95,
		Shift = 96,
		Alt = 97,
	}
}

script_enum! {
	/// Whether a key event is a press or a release. Vanilla's handlers act on `Up`.
	KeyState {
		Up = 0,
		Down = 1,
	}
}

script_enum! {
	/// `getFire()`. The original fills it with the key's state, so a press is
	/// `Repeated` and a release `Once` (see `key_queue`).
	KeyFire {
		Once = 0,
		Repeated = 1,
	}
}

impl Key {
	/// The key `offset` places after `self` (`Key::A.offset(2)` is `C`), if there is one.
	pub const fn offset(self, offset: i64) -> Option<Key> {
		Key::from_code(self.code() + offset)
	}
}
