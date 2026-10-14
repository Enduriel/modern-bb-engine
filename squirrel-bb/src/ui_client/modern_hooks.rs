//! Modern Hooks' connection screen, headless.
//!
//! Modern Hooks holds back vanilla's `root_state.onInit` (which makes `::Cursor`,
//! `::LoadingScreen` and the other UI globals) and `main_menu_state.onInit` until the UI
//! has loaded every mod's JS and CSS. Its JS side (`ModernHooksConnection.onConnection`
//! in `fps_module.js`) asks the script side for the files with `queryData`, loads them,
//! registers the mods' screens, then calls `resumeOnInit`, which runs both onInits.
//! Headless there are no files to load, so the handshake completes as soon as the
//! connection is answered.

use squirrel::{SQVM, SqStrongRef};

/// The screen name Modern Hooks connects its script side under.
pub(super) const SCREEN: &str = "ModernHooksConnection";

/// Answer the connection as the JS does: `queryData`, then `resumeOnInit`.
pub(super) fn on_connected(vm: &SQVM, receiver: &SqStrongRef) -> Result<(), String> {
	receiver.call_method(vm, "queryData", ()).map_err(|e| format!("{SCREEN}.queryData: {e}"))?;
	receiver.call_method(vm, "resumeOnInit", ()).map_err(|e| format!("{SCREEN}.resumeOnInit: {e}"))
}
