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
//! Headless there is no animation, so each callback follows on the next pump. Modules
//! with more behaviour than that get their own emulation ([`turn_sequence_bar`]), and so
//! do Modern Hooks' loader handshake ([`modern_hooks`]) and mods' late JS
//! ([`late_js`]). A real browser host replaces this module.

mod late_js;
mod modern_hooks;
mod turn_sequence_bar;

use squirrel::{SQVM, SqStrongRef};

use crate::UiBridge;
use turn_sequence_bar::TurnSequenceBar;

/// Call `method` on `receiver` if it defines it (a missing callback is fine: not every
/// screen implements every hook).
fn notify(vm: &SQVM, receiver: &SqStrongRef, method: &str) -> Result<(), String> {
	match receiver.call_method(vm, method, ()) {
		Err(e) if e.starts_with("no method") => Ok(()),
		Err(e) => Err(format!("{method}: {e}")),
		ok => ok,
	}
}

/// The headless browser side: per-module state that outlives a pump.
#[derive(Default)]
pub struct UiClient {
	turn_bar: TurnSequenceBar,
	/// The screen the turn order bar module is connected under.
	turn_bar_screen: Option<String>,
	/// What an error in a script callback does to the pump (see [`crate::ScriptErrors`]).
	script_errors: crate::ScriptErrors,
}

impl UiClient {
	pub fn new() -> Self {
		Self::default()
	}

	/// What an error in a script callback the pump calls does from now on.
	pub fn set_script_errors(&mut self, policy: crate::ScriptErrors) {
		self.script_errors = policy;
	}

	/// Answer everything recorded since the last pump: new connections first, then
	/// each `show`/`hide` in order, then the emulated modules' own callbacks. Returns
	/// the calls it consumed, for callers that drive input (e.g. pressing a button on
	/// a screen that was just shown).
	pub fn pump(&mut self, bridge: &UiBridge, vm: &SQVM) -> Result<Vec<crate::UiCall>, String> {
		for (screen, module) in bridge.take_new_connections() {
			match module {
				None => {
					if let Some(receiver) = bridge.receiver(&screen, None) {
						let connected = notify(vm, &receiver, "onScreenConnected").map_err(|e| format!("{screen}.{e}"));
						self.script_errors.settle(connected)?;
						if screen == modern_hooks::SCREEN {
							modern_hooks::on_connected(vm, &receiver)?;
						}
					}
				}
				Some(module) => {
					if let Some(receiver) = bridge.receiver(&screen, Some(&module)) {
						late_js::on_module_connected(vm, &screen, &module, &receiver)?;
					}
				}
			}
		}
		let calls = bridge.take_calls();
		for call in &calls {
			if call.module.as_deref() == Some(turn_sequence_bar::MODULE) {
				self.turn_bar_screen = Some(call.screen.clone());
				self.turn_bar.apply(vm, call);
			}
			let callback = match (call.method.as_str(), call.module.is_some()) {
				("show", false) => "onScreenShown",
				("hide", false) => "onScreenHidden",
				("show", true) => "onModuleShown",
				("hide", true) => "onModuleHidden",
				_ => continue,
			};
			if let Some(receiver) = bridge.receiver(&call.screen, call.module.as_deref()) {
				let notified = notify(vm, &receiver, callback).map_err(|e| format!("{}.{e}", call.screen));
				self.script_errors.settle(notified)?;
			}
		}
		if let Some(screen) = &self.turn_bar_screen {
			if let Some(receiver) = bridge.receiver(screen, Some(turn_sequence_bar::MODULE)) {
				self.turn_bar.advance(vm, &receiver)?;
			}
		}
		Ok(calls)
	}
}
