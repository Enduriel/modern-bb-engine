//! The engine's own settings file. The original keeps its settings in `config.cfg`
//! (binary, its own layout); ours is a text file of `key = value` lines ([`file`]) in
//! the system's configuration folder ([`dir`]), never the original's, with the video
//! mode under `video.*` ([`video`]) and room for engine-only options beside it.

pub mod dir;
pub mod file;
pub mod video;

use std::path::{Path, PathBuf};

use squirrel_bb::VideoMode;

use file::SettingsText;

/// The settings file's name in its folder.
const FILE_NAME: &str = "settings.cfg";

/// The settings file, where it is and what it says.
pub struct Settings {
	path: Option<PathBuf>,
	text: SettingsText,
}

impl Settings {
	/// The settings at the system's place for them (see [`dir`]).
	pub fn load_default() -> Settings {
		Settings::load(dir::settings_dir().map(|d| d.join(FILE_NAME)))
	}

	/// The settings at `path` (none at all without a path); a file that's missing or
	/// unreadable is an empty one.
	pub fn load(path: Option<PathBuf>) -> Settings {
		let text = path.as_deref().and_then(|p| std::fs::read_to_string(p).ok());
		Settings {
			path,
			text: text.map(|t| SettingsText::parse(&t)).unwrap_or_default(),
		}
	}

	pub fn path(&self) -> Option<&Path> {
		self.path.as_deref()
	}

	/// The video mode to start in, with what the file leaves out from `start`.
	pub fn video_mode(&self, start: VideoMode) -> VideoMode {
		video::read(&self.text, start)
	}

	/// The GPU memory for textures, in MiB (`render.texture_budget_mib`), if the file
	/// sets one; otherwise it follows the GPU (`bb_render::Gpu::texture_budget`).
	pub fn texture_budget_mib(&self) -> Option<u64> {
		self.text.get("render.texture_budget_mib")?.parse().ok().filter(|&mib| mib > 0)
	}

	/// Whether the FPS display shows from the start (`ui.show_fps = true`); it doesn't
	/// by default, as in the original.
	pub fn show_fps(&self) -> bool {
		self.text.get("ui.show_fps").is_some_and(|v| matches!(v, "true" | "1" | "on" | "yes"))
	}

	/// Keep `mode`, in the file at once.
	pub fn save_video_mode(&mut self, mode: &VideoMode) -> Result<(), String> {
		video::write(&mut self.text, mode);
		self.save()
	}

	fn save(&self) -> Result<(), String> {
		let Some(path) = &self.path else { return Ok(()) };
		if let Some(dir) = path.parent() {
			std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
		}
		std::fs::write(path, self.text.render()).map_err(|e| format!("{}: {e}", path.display()))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn saves_and_loads_the_video_mode() {
		let dir = std::env::temp_dir().join(format!("bb-settings-{}", std::process::id()));
		let path = dir.join("nested").join(FILE_NAME);
		let start = VideoMode::first_start(1920, 1080, 32);
		let mut settings = Settings::load(Some(path.clone()));
		assert_eq!(settings.video_mode(start), start, "no file yet");
		let mode = VideoMode {
			width: 1280,
			height: 720,
			fullscreen: false,
			..start
		};
		settings.save_video_mode(&mode).expect("save");
		assert_eq!(Settings::load(Some(path.clone())).video_mode(start), mode);
		assert_eq!(Settings::load(Some(path.clone())).texture_budget_mib(), None, "the GPU's by default");
		let text = std::fs::read_to_string(&path).expect("the file") + "render.texture_budget_mib = 256\n";
		std::fs::write(&path, text).expect("write");
		assert_eq!(Settings::load(Some(path.clone())).texture_budget_mib(), Some(256));
		assert!(!Settings::load(Some(path.clone())).show_fps(), "no FPS display by default");
		let text = std::fs::read_to_string(&path).expect("the file") + "ui.show_fps = true\n";
		std::fs::write(&path, text).expect("write");
		assert!(Settings::load(Some(path)).show_fps());
		let _ = std::fs::remove_dir_all(dir);
	}
}
