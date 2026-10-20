//! The FPS display: vanilla's own FPS module on the root screen, which the
//! original never shows, shown and fed by the game when the player turns it on.

use super::UiSession;
use crate::json::Json;

/// Shows or hides the module, and keeps it on the root screen the game's calls reach
/// (see the file).
const MODULE: &str = include_str!("fps_module.js");

impl UiSession {
	/// Show vanilla's FPS module on the root screen, or hide it again. Nothing of it
	/// reaches the page until it is first shown. The page remembers it: a root screen
	/// that has no container yet, or one made again, gets the module once it does.
	pub fn show_fps_module(&self, shown: bool) -> Result<(), String> {
		self.eval(&format!("{MODULE}('show', {shown})"))?;
		Ok(())
	}

	/// Show `text` as the frame rate, by the module's `setFPS`, called as the engine
	/// calls the root screen's modules (`callFromSQ`). Only while the module is shown
	/// and has its label (the page keeps `text` for when it does): `setFPS` throws
	/// otherwise.
	pub fn set_fps(&self, text: &str) {
		let value = Json::Str(text.into());
		match self.eval(&format!("{MODULE}('set', {})", value.to_text())) {
			Ok(Json::Bool(true)) => {}
			Ok(_) => return,
			Err(error) => {
				self.record("bridge", format!("ui: the FPS display: {error}"));
				return;
			}
		}
		let (vm, bridge) = (self.engine().vm(), self.engine().ui());
		let args = [Json::Str("RootScreen".into()), Json::Str("FPSModule".into()), Json::Str("setFPS".into()), value];
		self.protocol().trigger(vm, bridge, "callFromSQ", args);
	}
}
