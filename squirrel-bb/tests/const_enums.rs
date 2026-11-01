//! The engine's enums in the const table, as scripts use them (values: parity
//! `menu/consts`, the original's).

use squirrel::SQVM;
use squirrel_bb::install_natives;

fn vm() -> SQVM {
	let mut vm = SQVM::new(2048);
	install_natives(&mut vm);
	vm
}

#[test]
fn scripts_compiled_after_startup_get_the_enums_folded_in() {
	// The way a mod's hook reads them (Hardened's tactical key handler). Folded at
	// compile time, so a root slot of the same name doesn't change them.
	vm().run_script(
		r#"
			::KeyModifier <- { Control = 99 };
			assert(KeyModifier.Control == 2);
			assert(Key.A == 11 && Key.Escape == 41 && Key.ArrowLeft == 48 && Key.F24 == 94);
			assert(KeyFire.Repeated == 1);
			assert(Mouse.Wheel == 7 && MouseState.WheelDown == 4);
			assert(ShakeType.KnockUp == 7);
			assert(CloudType.Custom == 4 && AmbientLightingPreset.Desert == 7);
			"#,
		"const_enums",
	)
	.expect("the enums are constants");
}

#[test]
fn key_modifier_and_key_state_are_one_table_as_in_the_original() {
	vm().run_script(
		r#"
			local ct = getconsttable();
			assert(ct.KeyModifier == ct.KeyState);
			assert(KeyState.Down == 1 && KeyState.Up == 0 && KeyModifier.Shift == 1);
			assert(!("Key" in getroottable()));
			"#,
		"key_modifier_state",
	)
	.expect("KeyModifier and KeyState share one table");
}
