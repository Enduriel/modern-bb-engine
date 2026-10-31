//! Which folder `bb` keeps the saves in (and the mods' storages, which they read as they
//! boot). In order, the first there is:
//!
//! 1. `--saves DIR`;
//! 2. `BB_SAVES_DIR` (set and not empty);
//! 3. `saves.dir` in bb's settings file;
//! 4. the original game's own savegames folder ([`original`]), so the player sees the
//!    same saves in both: ours are in the original's format, which it lists and loads,
//!    and we load its own. On Windows it's always there; on Linux only with a Steam
//!    install whose Proton prefix the original has run in; never on macOS;
//! 5. bb's own folder (`squirrel_bb::platform::saves_dir`:
//!    `battle-brothers-modern/savegames` in the user's data folder).
//!
//! One bb at a time plays from a folder: a second one on it is told so and ends
//! ([`lock`]). The choice is said once in the log, with why. Only the saves can go to the
//! original's folder: bb's log and settings stay in its own. The tests keep every run's
//! saves in a folder of their own (`BB_SAVES_DIR` or `--saves`), never the player's.

#[cfg(windows)]
pub(crate) mod documents;
pub mod lock;
pub mod original;

use std::path::{Path, PathBuf};

use squirrel_bb::html_log::{self, Class};

use crate::settings::Settings;

/// The settings' key for the saves' folder.
pub const SAVES_DIR: &str = "saves.dir";

/// What decides the folder, each `None` when not given.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
	/// `--saves`.
	pub argument: Option<PathBuf>,
	/// `BB_SAVES_DIR`.
	pub env: Option<PathBuf>,
	/// The settings' `saves.dir`.
	pub setting: Option<PathBuf>,
	/// The original's savegames folder, or why it isn't used.
	pub original: Option<Result<PathBuf, String>>,
	/// bb's own folder (`None`: the system has no data folder).
	pub own: Option<PathBuf>,
}

/// The folder chosen and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
	pub dir: PathBuf,
	pub why: String,
}

/// The first folder [`Inputs`] gives, in the order above; `None` (saves in memory only)
/// without any.
pub fn choose(inputs: Inputs) -> Option<Chosen> {
	let chosen = |dir: PathBuf, why: &str| Some(Chosen { dir, why: why.to_string() });
	if let Some(dir) = inputs.argument {
		return chosen(dir, "--saves");
	}
	if let Some(dir) = inputs.env {
		return chosen(dir, "BB_SAVES_DIR");
	}
	if let Some(dir) = inputs.setting {
		return chosen(dir, &format!("{SAVES_DIR} in the settings"));
	}
	let not_original = match inputs.original {
		Some(Ok(dir)) => return chosen(dir, "the original game's savegames folder"),
		Some(Err(why)) => why,
		None => "no savegames folder of the original's".to_string(),
	};
	let dir = inputs.own?;
	chosen(dir, &format!("bb's own folder: {not_original}"))
}

/// The inputs for the install whose data folder is `data_dir`, with `--saves`'s folder
/// (`argument`): the environment's, the settings' and the system's.
pub fn inputs(argument: Option<PathBuf>, settings: &Settings, data_dir: &Path) -> Inputs {
	Inputs {
		argument,
		env: std::env::var_os("BB_SAVES_DIR").filter(|v| !v.is_empty()).map(PathBuf::from),
		setting: settings.saves_dir(),
		original: Some(original::savegames_dir(data_dir)),
		own: squirrel_bb::platform::own_saves_dir(),
	}
}

/// [`choose`] for the install at `data_dir`, said once in the log (and on stderr).
/// The original's folder is only looked for when nothing earlier names one.
pub fn choose_for(argument: Option<PathBuf>, settings: &Settings, data_dir: &Path) -> Option<Chosen> {
	let mut inputs = inputs(argument, settings, data_dir);
	if inputs.argument.is_some() || inputs.env.is_some() || inputs.setting.is_some() {
		inputs.original = None;
	}
	let chosen = choose(inputs);
	let line = match &chosen {
		Some(c) => format!("{} (from {})", c.dir.display(), c.why),
		None => "in memory only (no folder for them)".to_string(),
	};
	eprintln!("saves: {line}");
	html_log::line(Class::Info, "Saves", format!("saves: {line}"));
	chosen
}

#[cfg(test)]
mod tests {
	use super::*;

	fn all() -> Inputs {
		Inputs {
			argument: Some("/arg".into()),
			env: Some("/env".into()),
			setting: Some("/set".into()),
			original: Some(Ok("/orig".into())),
			own: Some("/own".into()),
		}
	}

	#[test]
	fn the_first_given_wins_in_order() {
		let dir = |i: Inputs| choose(i).map(|c| c.dir);
		let mut i = all();
		assert_eq!(dir(i.clone()), Some("/arg".into()));
		i.argument = None;
		assert_eq!(dir(i.clone()), Some("/env".into()));
		i.env = None;
		assert_eq!(dir(i.clone()), Some("/set".into()));
		i.setting = None;
		assert_eq!(dir(i.clone()), Some("/orig".into()));
		i.original = Some(Err("no Proton prefix".into()));
		let own = choose(i.clone()).unwrap();
		assert_eq!(own.dir, PathBuf::from("/own"));
		assert!(own.why.contains("no Proton prefix"), "{}", own.why);
		i.own = None;
		assert_eq!(choose(i), None);
	}
}
