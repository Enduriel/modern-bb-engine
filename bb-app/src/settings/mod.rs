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

	/// The folder the player keeps the saves in (`saves.dir`; see `bb_app::saves`),
	/// if the file names one.
	pub fn saves_dir(&self) -> Option<PathBuf> {
		self.text.get(crate::saves::SAVES_DIR).filter(|v| !v.is_empty()).map(PathBuf::from)
	}

	/// Keep `dir` as the data folder the player chose, in the file at once.
	pub fn keep_chosen_data_dir(&mut self, dir: &Path) -> Result<(), String> {
		let text = dir.to_str().ok_or_else(|| format!("{} isn't valid UTF-8, which the settings file needs", dir.display()))?;
		self.update(|t| t.set(CHOSEN_DATA_DIR, text))
	}

	/// The release version the player chose to ignore in the update notice
	/// (`update.ignored_version`; see `bb_app::update_check`), if any.
	pub fn ignored_update(&self) -> Option<&str> {
		self.text.get(crate::update_check::IGNORED_VERSION).filter(|v| !v.is_empty())
	}

	/// Keep `version` as the one the player ignored, in the file at once.
	pub fn keep_ignored_update(&mut self, version: &str) -> Result<(), String> {
		self.update(|t| t.set(crate::update_check::IGNORED_VERSION, version))
	}

	/// Keep what the scripts saved (`Settings.save`), in the file at once.
	pub fn keep(&mut self, saved: &SavedSettings) -> Result<(), String> {
		self.update(|t| {
			video::write(t, &saved.video);
			game::write(t, &saved.game);
		})
	}

	/// Make `change` to the file as it is now, and save it. Read again first: other
	/// parts of bb keep their own keys in it meanwhile (the update notice's ignored
	/// version), and the player may have edited it, so what was read at the start would
	/// lose their lines.
	fn update(&mut self, change: impl FnOnce(&mut SettingsText)) -> Result<(), String> {
		if let Some(text) = self.path.as_deref().and_then(|p| std::fs::read_to_string(p).ok()) {
			self.text = SettingsText::parse(&text);
		}
		change(&mut self.text);
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

	#[test]
	fn keeps_the_ignored_update_beside_what_another_writer_keeps() {
		let dir = std::env::temp_dir().join(format!("bb-settings-ignored-{}", std::process::id()));
		let path = dir.join(FILE_NAME);
		// The game's settings, read at the start; the update notice's, its own.
		let mut game = Settings::load(Some(path.clone()));
		let mut notice = Settings::load(Some(path.clone()));
		assert_eq!(notice.ignored_update(), None);
		notice.keep_ignored_update("0.3.0").expect("save");
		let start = VideoMode::first_start(1920, 1080, 32);
		game.keep(&SavedSettings { video: start, game: GameSettings::FRESH_INSTALL }).expect("save");
		let read = Settings::load(Some(path.clone()));
		assert_eq!(read.ignored_update(), Some("0.3.0"), "the game's save kept it");
		assert_eq!(read.video_mode(VideoMode::DEFAULT), start);
		notice.keep_ignored_update("0.4.0").expect("save");
		assert_eq!(Settings::load(Some(path.clone())).ignored_update(), Some("0.4.0"));
		assert_eq!(Settings::load(Some(path)).video_mode(VideoMode::DEFAULT), start, "and the notice the game's");
		let _ = std::fs::remove_dir_all(dir);
	}
}
