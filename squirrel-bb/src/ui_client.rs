//! A headless stand-in for the browser side of the UI: it answers the SQ→JS calls the
//! [`UiBridge`] records with the callbacks the game's JS screens send back, so the
//! script UI runs its normal lifecycle without a browser.
//!
//! The vanilla JS screens and modules all follow one pattern:
//!
//! * when a screen connects, the JS calls `onScreenConnected`;
//! * `show` and `hide` animate, then report `onScreenShown` / `onScreenHidden` (a module
//!   reports `onModuleShown` / `onModuleHidden`).
//!
//! Headless there is no animation, so each callback follows on the next pump. A real
//! browser host replaces this module.

use squirrel::{SQVM, SqStrongRef};

use crate::UiBridge;

/// Call `method` on `receiver` if it defines it (a missing callback is fine: not every
/// screen implements every hook).
fn notify(vm: &SQVM, receiver: &SqStrongRef, method: &str) -> Result<(), String> {
	match receiver.call_method(vm, method, ()) {
		Err(e) if e.starts_with("no method") => Ok(()),
		Err(e) => Err(format!("{method}: {e}")),
		ok => ok,
	}
}

/// Answer everything recorded since the last pump: new connections first, then each
/// `show`/`hide` in order. Returns the calls it consumed, for callers that drive input
/// (e.g. pressing a button on a screen that was just shown).
pub fn pump(bridge: &UiBridge, vm: &SQVM) -> Result<Vec<crate::UiCall>, String> {
	for (screen, module) in bridge.take_new_connections() {
		if module.is_none() {
			if let Some(receiver) = bridge.receiver(&screen, None) {
				notify(vm, &receiver, "onScreenConnected").map_err(|e| format!("{screen}.{e}"))?;
			}
		}
	}
	let calls = bridge.take_calls();
	for call in &calls {
		let callback = match (call.method.as_str(), call.module.is_some()) {
			("show", false) => "onScreenShown",
			("hide", false) => "onScreenHidden",
			("show", true) => "onModuleShown",
			("hide", true) => "onModuleHidden",
			_ => continue,
		};
		if let Some(receiver) = bridge.receiver(&call.screen, call.module.as_deref()) {
			notify(vm, &receiver, callback).map_err(|e| format!("{}.{e}", call.screen))?;
		}
	}
	Ok(calls)
}
