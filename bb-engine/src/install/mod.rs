//! Finding the player's copy of the game: the data folder bb runs (`<game>/data`, with
//! the `data_*.dat` archives). The first of these that holds the game ([`layout`])
//! is used:
//!
//! 1. `--data DIR`, then `BB_DATA_DIR`: the developer's overrides. One that names no
//!    install stops bb with that error; it never falls through to another install.
//! 2. The folder the player chose when none was found (kept in bb's settings; the
//!    caller passes it). One that no longer holds the game (moved, uninstalled) is
//!    passed over, and the log says so.
//! 3. The stores' installs, in [`STORES`]' order: Steam's libraries ([`steam`]).
//!
//! Nothing found: the caller tells the player where it looked ([`NotFound`]) and lets
//! them choose the folder.

pub mod layout;
pub mod steam;
#[cfg(test)]
mod test_tree;

use std::fmt;
use std::path::{Path, PathBuf};

/// The variable naming the data folder (a developer's override).
pub const DATA_DIR_VAR: &str = "BB_DATA_DIR";

/// Where an install was found, or looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
	/// `--data DIR`.
	Argument,
	/// `BB_DATA_DIR`.
	Environment,
	/// The folder the player chose earlier.
	Remembered,
	/// A store's install.
	Store(Store),
}

/// The stores whose installs bb finds by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
	Steam,
	// Gog: GOG Galaxy's and the offline installers' copies. Not yet: the crate that finds
	// them is being chosen. It goes after Steam in `STORES`.
}

/// The stores looked in, in order: with several installs, the first found wins.
pub const STORES: &[Store] = &[Store::Steam];

impl fmt::Display for Source {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Source::Argument => "--data",
			Source::Environment => DATA_DIR_VAR,
			Source::Remembered => "the folder chosen earlier",
			Source::Store(Store::Steam) => "Steam",
		})
	}
}

/// A place looked in, and what was there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Looked {
	pub source: Source,
	/// Where (`Steam library /path`, a folder).
	pub place: String,
	/// What was found there instead of the game.
	pub outcome: String,
}

impl Looked {
	pub fn new(source: Source, place: impl Into<String>, outcome: impl Into<String>) -> Looked {
		Looked { source, place: place.into(), outcome: outcome.into() }
	}
}

impl fmt::Display for Looked {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}: {}", self.place, self.outcome)
	}
}

/// The install found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
	/// Its data folder.
	pub data_dir: PathBuf,
	pub source: Source,
	/// The places passed over before it (a remembered folder that's gone), for the log.
	pub passed_over: Vec<Looked>,
}

/// No install to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotFound {
	/// A developer's override names a folder without the game in it.
	BadOverride(Looked),
	/// None anywhere: every place looked in, in order.
	Nowhere(Vec<Looked>),
}

impl fmt::Display for NotFound {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			NotFound::BadOverride(looked) => write!(f, "{} names no Battle Brothers install: {looked}", looked.source),
			NotFound::Nowhere(looked) => {
				f.write_str("Battle Brothers wasn't found. Looked in:")?;
				for place in looked {
					write!(f, "\n- {place}")?;
				}
				Ok(())
			}
		}
	}
}

/// What the lookup goes through, the overrides and remembered folder as given.
#[derive(Clone, Debug, Default)]
pub struct Lookup {
	/// `--data`.
	pub argument: Option<PathBuf>,
	/// `BB_DATA_DIR`.
	pub environment: Option<PathBuf>,
	/// The folder the player chose earlier.
	pub remembered: Option<PathBuf>,
	/// Steam's folder (`BB_STEAM_DIR`); found by `steamlocate` without one.
	pub steam_dir: Option<PathBuf>,
}

impl Lookup {
	/// The lookup with `--data`'s folder and the remembered one, the rest from the
	/// environment.
	pub fn from_env(argument: Option<PathBuf>, remembered: Option<PathBuf>) -> Lookup {
		Lookup {
			argument,
			environment: env_dir(DATA_DIR_VAR),
			remembered,
			steam_dir: env_dir(steam::STEAM_DIR_VAR),
		}
	}

	/// The install to run (see the module's notes for the order).
	pub fn find(&self) -> Result<Install, NotFound> {
		let overrides = [(Source::Argument, &self.argument), (Source::Environment, &self.environment)];
		for (source, dir) in overrides {
			let Some(dir) = dir else { continue };
			return match layout::data_dir_in(dir) {
				Some(data_dir) => Ok(Install { data_dir, source, passed_over: Vec::new() }),
				None => Err(NotFound::BadOverride(Looked::new(source, dir.display().to_string(), no_game()))),
			};
		}
		let mut looked = Vec::new();
		if let Some(dir) = &self.remembered {
			match layout::data_dir_in(dir) {
				Some(data_dir) => return Ok(Install { data_dir, source: Source::Remembered, passed_over: looked }),
				None => looked.push(Looked::new(Source::Remembered, dir.display().to_string(), no_game())),
			}
		}
		for &store in STORES {
			let found = match store {
				Store::Steam => steam::data_dirs(self.steam_dir.as_deref(), &mut looked),
			};
			if let Some(data_dir) = found.into_iter().next() {
				return Ok(Install { data_dir, source: Source::Store(store), passed_over: looked });
			}
		}
		Err(NotFound::Nowhere(looked))
	}
}

/// What a folder without the game has.
fn no_game() -> String {
	format!("no {} there (nor in a data folder in it)", layout::MARKER)
}

/// A variable's folder, if it's set and not empty.
fn env_dir(var: &str) -> Option<PathBuf> {
	std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The game's data folder in Steam's libraries (`BB_STEAM_DIR`'s, or the Steam folders
/// found): the tests' and tools' install when `BB_DATA_DIR` names none.
pub fn steam_data_dir() -> Option<PathBuf> {
	steam::data_dirs(env_dir(steam::STEAM_DIR_VAR).as_deref(), &mut Vec::new()).into_iter().next()
}

/// The folder a player picked, as the data folder to run, or why it isn't the game's.
pub fn chosen(folder: &Path) -> Result<PathBuf, String> {
	layout::data_dir_in(folder).ok_or_else(|| {
		format!(
			"{} isn't Battle Brothers' folder: there's no data{}{} in it. Choose the folder the game is installed in.",
			folder.display(),
			std::path::MAIN_SEPARATOR,
			layout::MARKER
		)
	})
}

#[cfg(test)]
mod tests {
	use super::test_tree::Tree;
	use super::*;

	/// The lookup over the tree's folders: no Steam unless `steam` names one.
	fn lookup(argument: Option<&Path>, environment: Option<&Path>, remembered: Option<&Path>, steam: &Path) -> Lookup {
		Lookup {
			argument: argument.map(Path::to_path_buf),
			environment: environment.map(Path::to_path_buf),
			remembered: remembered.map(Path::to_path_buf),
			steam_dir: Some(steam.to_path_buf()),
		}
	}

	#[test]
	fn the_overrides_come_first_then_the_remembered_folder_then_steam() {
		let tree = Tree::new("order");
		let (arg, env, kept) = (tree.game("arg"), tree.game("env"), tree.game("kept"));
		let steam = tree.steam("Steam", "Battle Brothers", true);
		let steam_data = steam.join("steamapps/common/Battle Brothers/data");
		let found = |l: Lookup| l.find().map(|i| (i.data_dir, i.source));
		let all = lookup(Some(&arg), Some(&env), Some(&kept), &steam);
		assert_eq!(found(all), Ok((arg.join("data"), Source::Argument)));
		assert_eq!(found(lookup(None, Some(&env), Some(&kept), &steam)), Ok((env.join("data"), Source::Environment)));
		assert_eq!(found(lookup(None, None, Some(&kept), &steam)), Ok((kept.join("data"), Source::Remembered)));
		assert_eq!(found(lookup(None, None, None, &steam)), Ok((steam_data, Source::Store(Store::Steam))));
	}

	#[test]
	fn a_bad_override_stops_the_lookup() {
		let tree = Tree::new("bad_override");
		let steam = tree.steam("Steam", "Battle Brothers", true);
		let empty = tree.root().join("empty");
		std::fs::create_dir_all(&empty).unwrap();
		match lookup(Some(&empty), None, None, &steam).find() {
			Err(NotFound::BadOverride(looked)) => assert_eq!(looked.source, Source::Argument),
			other => panic!("{other:?}"),
		}
	}

	#[test]
	fn a_remembered_folder_thats_gone_is_passed_over() {
		let tree = Tree::new("gone");
		let steam = tree.steam("Steam", "Battle Brothers", true);
		let gone = tree.root().join("moved away");
		let install = lookup(None, None, Some(&gone), &steam).find().expect("Steam's");
		assert_eq!(install.source, Source::Store(Store::Steam));
		assert_eq!(install.passed_over.len(), 1, "{:?}", install.passed_over);
		assert_eq!(install.passed_over[0].source, Source::Remembered);
	}

	#[test]
	fn nothing_found_says_where_it_looked() {
		let tree = Tree::new("nowhere");
		let gone = tree.root().join("moved away");
		let no_steam = tree.root().join("no steam");
		std::fs::create_dir_all(&no_steam).unwrap();
		let Err(NotFound::Nowhere(looked)) = lookup(None, None, Some(&gone), &no_steam).find() else { panic!("found one") };
		let sources: Vec<Source> = looked.iter().map(|l| l.source).collect();
		assert_eq!(sources, [Source::Remembered, Source::Store(Store::Steam)]);
		let text = NotFound::Nowhere(looked).to_string();
		assert!(text.starts_with("Battle Brothers wasn't found. Looked in:\n- "), "{text}");
	}

	#[test]
	fn a_chosen_folder_is_checked() {
		let tree = Tree::new("chosen");
		let game = tree.game("Battle Brothers");
		assert_eq!(chosen(&game), Ok(game.join("data")));
		let error = chosen(tree.root()).expect_err("not the game");
		assert!(error.contains("isn't Battle Brothers' folder"), "{error}");
	}

	/// This machine's Steam install, as the tests and tools find it.
	#[test]
	fn this_machines_steam_install_is_found() {
		let mut looked = Vec::new();
		let Some(data) = steam::data_dirs(None, &mut looked).into_iter().next() else {
			return crate::harness::skip(format!("no Steam install of Battle Brothers on this machine: {looked:?}"));
		};
		assert!(layout::is_data_dir(&data), "{}", data.display());
		assert!(data.ends_with("data"), "{}", data.display());
	}
}
