//! The engine's own settings file. The original keeps its settings in `config.cfg`
//! (binary, its own layout); ours is a text file of `key = value` lines ([`file`]) in
//! the system's configuration folder ([`dir`]), never the original's, with the video
//! mode under `video.*` ([`video`]), what else the original keeps (gameplay, controls,
//! sound) under `gameplay.*`, `controls.*` and `audio.*` ([`game`]), and room for
//! engine-only options beside them.

pub mod dir;
pub mod file;
pub mod game;
pub mod video;

use std::path::{Path, PathBuf};

use squirrel_bb::{GameSettings, SavedSettings, VideoMode};

use file::SettingsText;

/// The settings file's name in its folder.
const FILE_NAME: &str = "settings.cfg";

/// The key of the data folder the player chose (or named by hand: the not-found window
/// tells them this key).
pub const CHOSEN_DATA_DIR: &str = "install.data_dir";

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

	/// The gameplay, control and sound settings to start with; what the file leaves out
	/// is a fresh install's.
	pub fn game_settings(&self) -> GameSettings {
		game::read(&self.text, GameSettings::FRESH_INSTALL)
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

	/// Whether the game draws its mouse cursor into its frames, as the original does
	/// (`ui.software_cursor = true`), rather than have the system show it (the default:
	/// a mouse move then needs no frame drawn; see `game_cursor`).
	pub fn software_cursor(&self) -> bool {
		self.text.get("ui.software_cursor").is_some_and(|v| matches!(v, "true" | "1" | "on" | "yes"))
	}

	/// Whether a Retina Mac draws the world and battles at the display's pixels
	/// (`render.retina_world = true`), rather than at the game's screen's size enlarged
	/// pixel for pixel (the default: a quarter of the GPU's work; see `world_scale`).
	pub fn retina_world(&self) -> bool {
		self.text.get("render.retina_world").is_some_and(|v| matches!(v, "true" | "1" | "on" | "yes"))
	}

	/// The game's data folder the player chose when bb found none
	/// (`install.data_dir`; see `bb_app::install`).
	pub fn chosen_data_dir(&self) -> Option<PathBuf> {
		self.text.get(CHOSEN_DATA_DIR).filter(|v| !v.is_empty()).map(PathBuf::from)
	}

	/// Keep `dir` as the data folder the player chose, in the file at once.
	pub fn keep_chosen_data_dir(&mut self, dir: &Path) -> Result<(), String> {
		let text = dir.to_str().ok_or_else(|| format!("{} isn't valid UTF-8, which the settings file needs", dir.display()))?;
		self.text.set(CHOSEN_DATA_DIR, text);
		self.save()
	}

	/// Keep what the scripts saved (`Settings.save`), in the file at once.
	pub fn keep(&mut self, saved: &SavedSettings) -> Result<(), String> {
		video::write(&mut self.text, &saved.video);
		game::write(&mut self.text, &saved.game);
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
		assert_eq!(settings.game_settings(), GameSettings::FRESH_INSTALL, "a fresh install's");
		let mut game = GameSettings::FRESH_INSTALL;
		game.set_gameplay("DontAutoEndTurns", true);
		game.volumes[0] = 0.5;
		settings.keep(&SavedSettings { video: mode, game }).expect("save");
		assert_eq!(Settings::load(Some(path.clone())).video_mode(start), mode);
		assert_eq!(Settings::load(Some(path.clone())).game_settings(), game);
		assert_eq!(Settings::load(Some(path.clone())).texture_budget_mib(), None, "the GPU's by default");
		let text = std::fs::read_to_string(&path).expect("the file") + "render.texture_budget_mib = 256\n";
		std::fs::write(&path, text).expect("write");
		assert_eq!(Settings::load(Some(path.clone())).texture_budget_mib(), Some(256));
		assert!(!Settings::load(Some(path.clone())).show_fps(), "no FPS display by default");
		let text = std::fs::read_to_string(&path).expect("the file") + "ui.show_fps = true\n";
		std::fs::write(&path, text).expect("write");
		assert!(Settings::load(Some(path.clone())).show_fps());
		assert!(!Settings::load(Some(path.clone())).software_cursor(), "the system shows the cursor by default");
		let text = std::fs::read_to_string(&path).expect("the file") + "ui.software_cursor = true\n";
		std::fs::write(&path, text).expect("write");
		assert!(Settings::load(Some(path.clone())).software_cursor());
		assert!(!Settings::load(Some(path.clone())).retina_world(), "the world at the screen's size by default");
		let text = std::fs::read_to_string(&path).expect("the file") + "render.retina_world = true\n";
		std::fs::write(&path, text).expect("write");
		assert!(Settings::load(Some(path.clone())).retina_world());
		assert_eq!(Settings::load(Some(path.clone())).chosen_data_dir(), None, "none chosen yet");
		let chosen = Path::new("/games/Battle Brothers/data");
		Settings::load(Some(path.clone())).keep_chosen_data_dir(chosen).expect("save");
		assert_eq!(Settings::load(Some(path.clone())).chosen_data_dir().as_deref(), Some(chosen));
		assert!(Settings::load(Some(path)).retina_world(), "the rest kept");
		let _ = std::fs::remove_dir_all(dir);
	}
}
