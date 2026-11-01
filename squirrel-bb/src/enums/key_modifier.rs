//! `KeyModifier`: the modifier keys held, a mask (`getModifier()`).

use std::ops::{BitOr, BitOrAssign};

use super::Key;

/// The modifier keys held: Shift, Control and Alt, combined (Shift+Control is 3), and
/// the host's [`COMMAND`](Self::COMMAND), which the scripts never see.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyModifier(i64);

impl KeyModifier {
	pub const NONE: Self = KeyModifier(0);
	pub const SHIFT: Self = KeyModifier(1);
	pub const CONTROL: Self = KeyModifier(2);
	pub const ALT: Self = KeyModifier(4);
	/// The Mac's Command key: `bb` sets it on macOS so the UI page gets it (`metaKey`,
	/// and Cmd+C and the other editing keys). The original had no such key, so
	/// [`for_scripts`](Self::for_scripts) leaves it out.
	pub const COMMAND: Self = KeyModifier(8);

	/// The members the scripts see (`COMMAND` isn't one).
	pub const MEMBERS: &'static [(&'static str, i64)] = &[
		("None", Self::NONE.0),
		("Shift", Self::SHIFT.0),
		("Control", Self::CONTROL.0),
		("Alt", Self::ALT.0),
	];

	/// Whether every key held in `other` is held in `self`.
	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}

	/// `self` with `other`'s keys held (`on`) or not.
	pub const fn with(self, other: Self, on: bool) -> Self {
		KeyModifier(if on { self.0 | other.0 } else { self.0 & !other.0 })
	}

	/// The mask `getModifier()` returns: Shift, Control and Alt.
	pub const fn for_scripts(self) -> i64 {
		self.0 & !Self::COMMAND.0
	}

	/// The modifier `key` is, if it's one of the modifier keys.
	pub const fn of_key(key: Key) -> Option<Self> {
		match key {
			Key::Shift => Some(Self::SHIFT),
			Key::Control => Some(Self::CONTROL),
			Key::Alt => Some(Self::ALT),
			_ => None,
		}
	}
}

impl BitOr for KeyModifier {
	type Output = Self;

	fn bitor(self, other: Self) -> Self {
		KeyModifier(self.0 | other.0)
	}
}

impl BitOrAssign for KeyModifier {
	fn bitor_assign(&mut self, other: Self) {
		self.0 |= other.0;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_scripts_never_see_command() {
		assert_eq!(KeyModifier::COMMAND.for_scripts(), 0);
		assert_eq!((KeyModifier::COMMAND | KeyModifier::CONTROL).for_scripts(), 2);
		assert_eq!((KeyModifier::SHIFT | KeyModifier::CONTROL | KeyModifier::ALT).for_scripts(), 7);
	}

	#[test]
	fn keys_held_and_let_go() {
		let held = KeyModifier::SHIFT | KeyModifier::ALT;
		assert!(held.contains(KeyModifier::ALT));
		assert!(!held.contains(KeyModifier::CONTROL));
		assert_eq!(held.with(KeyModifier::SHIFT, false), KeyModifier::ALT);
		assert_eq!(KeyModifier::of_key(Key::Control), Some(KeyModifier::CONTROL));
		assert_eq!(KeyModifier::of_key(Key::C), None);
	}
}
