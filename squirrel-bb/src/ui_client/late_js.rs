//! Mods' JS that calls back into the script side as soon as it runs, headless.
//!
//! MSU's `connect_screens.js`, registered as late JS (it runs after Modern Hooks has
//! resumed the boot), asks the main menu module's script side to `connectBackend`,
//! which MSU hooks to connect its own screens (settings, popup). Without it, MSU's
//! settings screen has no handle when a campaign starts. Headless, the call follows on
//! the pump after the module connects; without MSU the module has no such method.

use squirrel::{SQVM, SqStrongRef};

/// Answer a new module connection as the mods' late JS does.
pub(super) fn on_module_connected(vm: &SQVM, screen: &str, module: &str, receiver: &SqStrongRef) -> Result<(), String> {
	if screen == "MainMenuScreen" && module == "MainMenuModule" {
		return super::notify(vm, receiver, "connectBackend").map_err(|e| format!("{screen}.{module}.{e}"));
	}
	Ok(())
}
