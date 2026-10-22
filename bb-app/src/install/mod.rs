//! Which copy of the game `bb` plays: the lookup (`bb_engine::install`: `--data`,
//! `BB_DATA_DIR`, the folder the player chose, Steam), with the folder the player
//! chose kept in bb's settings (`install.data_dir`), and the player asked for the
//! folder when none is found.
//!
//! Only a folder the player chose is kept, not one found: a store's install is found
//! again each start (in milliseconds), so a game Steam moved to another library is
//! followed there, and the settings hold only what the player decided.
//!
//! Asking: with the UI, a window of bb's own ([`picker`]); without it (a developer's
//! build), the message box saying where bb looked. Either way the player is told how to
//! name the folder by hand ([`by_hand`]): bb finds only Steam's install by itself, so
//! a GOG copy, or any other, is named in the settings file.

#[cfg(feature = "ui")]
pub mod picker;
pub mod relaunch;

use std::path::{Path, PathBuf};

use bb_engine::install::{Install, Looked, Lookup, NotFound};
use squirrel_bb::html_log::{self, Class};

use crate::settings::Settings;

/// Find the install to play: `--data`'s folder (`argument`), else as the lookup goes on
/// with the folder chosen earlier (`chosen`: the settings', or the one the player just
/// chose when it couldn't be kept). Says which, and from where, in the log.
pub fn find(argument: Option<PathBuf>, chosen: Option<PathBuf>) -> Result<Install, NotFound> {
	let install = Lookup::from_env(argument, chosen).find()?;
	for passed in &install.passed_over {
		log(Class::Warning, &format!("passed over {} ({}): {}", passed.source, passed.place, passed.outcome));
	}
	log(Class::Info, &format!("the game's data: {} (from {})", install.data_dir.display(), install.source));
	Ok(install)
}

/// How to name the game's folder by hand: the line to add to the settings file at
/// `settings_file`, with an example, read at the next start. `None` without a settings
/// file (no home folder).
pub fn by_hand(settings_file: Option<&Path>) -> Option<String> {
	let file = settings_file?;
	let key = crate::settings::CHOSEN_DATA_DIR;
	Some(format!(
		"To name the folder yourself (a GOG copy, or any other bb doesn't find), add this line to \
		 {}, creating the file if it isn't there:\n\n{key} = <the game's data folder>\n\nfor example\n\n\
		 {key} = {}\n\nbb reads it the next time it starts.",
		file.display(),
		example_data_dir()
	))
}

/// A data folder where a player's copy might be on this system, for the examples. On
/// macOS, where the game has no build of its own, a copy of the Windows game's files in
/// the player's home folder.
pub fn example_data_dir() -> &'static str {
	if cfg!(windows) {
		r"C:\GOG Games\Battle Brothers\data"
	} else if cfg!(target_os = "macos") {
		"/Users/you/Games/Battle Brothers/data"
	} else {
		"/home/you/GOG Games/Battle Brothers/data"
	}
}

/// What the player chose, after nothing was found.
pub enum Asked {
	/// This data folder: kept in the settings (or not, and why), and bb starts again.
	Chosen { data_dir: PathBuf, kept: Result<(), String> },
	/// They closed the window.
	Closed,
}

/// Ask the player for the game's folder (nothing was found in `looked`), and keep the
/// one they choose in `settings`.
#[cfg(feature = "ui")]
pub fn ask(looked: &[Looked], settings: &mut Settings, options: picker::Options) -> Result<Asked, String> {
	log(Class::Warning, &NotFound::Nowhere(looked.to_vec()).to_string());
	let Some(data_dir) = picker::ask(looked, settings.path(), options)? else {
		log(Class::Info, "no game folder chosen: the window was closed");
		return Ok(Asked::Closed);
	};
	log(Class::Info, &format!("the player chose the game's data: {}", data_dir.display()));
	let kept = settings.keep_chosen_data_dir(&data_dir);
	if let Err(e) = &kept {
		log(Class::Warning, &format!("the folder chosen can't be kept for next time: {e}"));
	}
	Ok(Asked::Chosen { data_dir, kept })
}

/// Without the UI there's no window to ask in: the error says where bb looked and how
/// to name the folder.
#[cfg(not(feature = "ui"))]
pub fn ask(looked: &[Looked], settings: &mut Settings, _options: ()) -> Result<Asked, String> {
	let mut text = format!(
		"{}\n\nStart bb with --data and the game's data folder (the `data` folder in Battle Brothers' \
		 folder).",
		NotFound::Nowhere(looked.to_vec())
	);
	if let Some(by_hand) = by_hand(settings.path()) {
		text.push_str(&format!("\n\n{by_hand}"));
	}
	Err(text)
}

/// The arguments the relaunched bb needs beyond this run's: the folder, if it couldn't
/// be kept in the settings (where the lookup finds it otherwise). As `--chosen-data`,
/// not `--data`: it's the player's choice, so a folder gone by then is passed over and
/// asked for again, as the settings' would be, not a command line's error.
pub fn relaunch_arguments(asked: &Asked) -> Vec<std::ffi::OsString> {
	match asked {
		Asked::Chosen { data_dir, kept: Err(_) } => vec!["--chosen-data".into(), data_dir.into()],
		_ => Vec::new(),
	}
}

/// A line in the log (and on stderr, as bb's other lines).
fn log(class: Class, text: &str) {
	eprintln!("install: {text}");
	html_log::line(class, "Install", text);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_folder_not_kept_is_passed_on_as_the_players_choice() {
		let chosen = |kept| Asked::Chosen { data_dir: PathBuf::from("/games/BB/data"), kept };
		assert_eq!(relaunch_arguments(&chosen(Err("read-only".into()))), ["--chosen-data", "/games/BB/data"]);
		assert!(relaunch_arguments(&chosen(Ok(()))).is_empty());
		assert!(relaunch_arguments(&Asked::Closed).is_empty());
	}
}
