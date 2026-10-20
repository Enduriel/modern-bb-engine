//! The FPS display: vanilla's own FPS module on the root screen, which the
//! original never shows, shown and fed by the game when the player turns it on.

use super::UiSession;
use crate::json::Json;

/// Shows or hides the module, given `true` or `false` (see the file).
const SHOW_MODULE: &str = include_str!("fps_module.js");

impl UiSession {
	/// Show vanilla's FPS module on the root screen, or hide it again. Nothing of it
	/// reaches the page until it is first shown.
	pub fn show_fps_module(&self, shown: bool) -> Result<(), String> {
		let there = self.eval(&format!("{SHOW_MODULE}({shown})"))?;
		if there != Json::Bool(true) {
			return Err("the page has no root screen with an FPS module".into());
		}
		Ok(())
	}

	/// Show `text` as the frame rate, by the module's `setFPS`, called as the engine
	/// calls the root screen's modules (`callFromSQ`). Only while the module is shown:
	/// it has no label otherwise.
	pub fn set_fps(&self, text: &str) {
		let (vm, bridge) = (self.engine().vm(), self.engine().ui());
		let args = ["RootScreen", "FPSModule", "setFPS", text].map(|s| Json::Str(s.into()));
		self.protocol().trigger(vm, bridge, "callFromSQ", args);
	}
}
