//! The enums the original engine registers in the const table (`getconsttable()`), all
//! nine of them, with the original's values (probed: parity `menu/consts`). Each value
//! is written once, here: the const table is filled from the same definitions the host
//! uses (a key's code, a mouse event's state, a shake's type).
//!
//! They must be in place before any script compiles: the compiler folds `Key.A` into
//! the literal `11`, and a script compiled without the const looks `Key` up in the root
//! table at run time, where the original has no such slot.
//!
//! The original binds `KeyModifier` and `KeyState` to one table holding both enums'
//! members (`KeyModifier.Down` is 1, `KeyState.Shift` is 1). Here they are two types,
//! and only the table is shared.

mod key_modifier;
mod keys;
mod mouse;
mod scene;

pub use key_modifier::KeyModifier;
pub use keys::{Key, KeyFire, KeyState};
pub use mouse::{Mouse, MouseState};
pub use scene::{AmbientLightingPreset, CloudType, ShakeType};

use squirrel::{SQVM, SqTable};

/// An enum the scripts see as a const table of `name = code` members: an `i64` enum
/// with `MEMBERS` (for the table), `code` and `from_code` (for a value a script passed,
/// which may be none of them).
macro_rules! script_enum {
	(
		$(#[$meta:meta])*
		$name:ident { $($(#[$vmeta:meta])* $variant:ident = $code:literal),+ $(,)? }
	) => {
		$(#[$meta])*
		#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
		#[repr(i64)]
		pub enum $name {
			$($(#[$vmeta])* $variant = $code),+
		}

		impl $name {
			/// Every member as the scripts see it: name and code.
			pub const MEMBERS: &'static [(&'static str, i64)] = &[$((stringify!($variant), $code)),+];

			/// The code the scripts see.
			pub const fn code(self) -> i64 {
				self as i64
			}

			/// The member with `code`, if any.
			pub const fn from_code(code: i64) -> Option<Self> {
				match code {
					$($code => Some(Self::$variant),)+
					_ => None,
				}
			}
		}
	};
}
pub(crate) use script_enum;

/// Register every enum in the const table. Call before any script compiles.
pub(crate) fn install(vm: &SQVM) {
	let key_modifier_and_state: Vec<(&str, i64)> =
		KeyModifier::MEMBERS.iter().chain(KeyState::MEMBERS).copied().collect();
	let mut consts = SqTable::const_table(vm);
	for (names, members) in [
		(&["AmbientLightingPreset"][..], AmbientLightingPreset::MEMBERS),
		(&["CloudType"], CloudType::MEMBERS),
		(&["Key"], Key::MEMBERS),
		(&["KeyFire"], KeyFire::MEMBERS),
		(&["KeyModifier", "KeyState"], &key_modifier_and_state),
		(&["Mouse"], Mouse::MEMBERS),
		(&["MouseState"], MouseState::MEMBERS),
		(&["ShakeType"], ShakeType::MEMBERS),
	] {
		let mut e = SqTable::new(vm);
		for (k, v) in members {
			e.set(k, *v);
		}
		let e = e.to_strong();
		for name in names {
			consts.set(name, e.clone());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn codes_go_both_ways() {
		assert_eq!(Key::Escape.code(), 41);
		assert_eq!(Key::from_code(41), Some(Key::Escape));
		assert_eq!(Key::from_code(98), None);
		assert_eq!(ShakeType::from_code(-1), None);
		for &(name, code) in Key::MEMBERS {
			assert_eq!(Key::from_code(code).map(|k| format!("{k:?}")), Some(name.to_string()));
		}
	}
}
