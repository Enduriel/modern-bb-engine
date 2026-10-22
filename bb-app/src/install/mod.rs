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
//! build), the message box saying where bb looked and to start it with `--data`.

#[cfg(feature = "ui")]
pub mod picker;
pub mod relaunch;

use std::path::PathBuf;

use bb_engine::install::{Install, Looked, Lookup, NotFound};
use squirrel_bb::html_log::{self, Class};

use crate::settings::Settings;

/// Find the install to play: `--data`'s folder (`argument`), else as the lookup goes on
/// with the folder chosen earlier from `settings`. Says which, and from where, in the log.
pub fn find(argument: Option<PathBuf>, settings: &Settings) -> Result<Install, NotFound> {
	let install = Lookup::from_env(argument, settings.chosen_data_dir()).find()?;
	for passed in &install.passed_over {
		log(Class::Warning, &format!("passed over {} ({}): {}", passed.source, passed.place, passed.outcome));
	}
	log(Class::Info, &format!("the game's data: {} (from {})", install.data_dir.display(), install.source));
	Ok(install)
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
	let Some(data_dir) = picker::ask(looked, options)? else {
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
pub fn ask(looked: &[Looked], _settings: &mut Settings, _options: ()) -> Result<Asked, String> {
	Err(format!(
		"{}\n\nStart bb with --data and the game's data folder (the `data` folder in the folder Battle Brothers \
		 is installed in).",
		NotFound::Nowhere(looked.to_vec())
	))
}

/// The arguments the relaunched bb needs beyond this run's: the folder, if it couldn't
/// be kept in the settings (where the lookup finds it otherwise).
pub fn relaunch_arguments(asked: &Asked) -> Vec<std::ffi::OsString> {
	match asked {
		Asked::Chosen { data_dir, kept: Err(_) } => vec!["--data".into(), data_dir.into()],
		_ => Vec::new(),
	}
}

/// A line in the log (and on stderr, as bb's other lines).
fn log(class: Class, text: &str) {
	eprintln!("install: {text}");
	html_log::line(class, "Install", text);
}
