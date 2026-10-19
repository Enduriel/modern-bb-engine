//! Steam for the windowed game: on a Steam install, the game's achievements and stats go
//! to the player's Steam account, as the original's do. On any other
//! install (GOG, a copy), or with `--no-steam`, Steam's library is never loaded and the
//! game runs as before (no achievement unlocks, stats for the run only).
//!
//! The decision ([`decide`]) only looks at folders and files. Then [`start`] loads the
//! library and starts Steam's API; any failure (no library beside bb, Steam not running
//! or not logged in) turns achievements off, said once in the log, and the game runs on.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use bb_steam::client::Steam;
use squirrel_bb::UserStats;

/// What the player asked of Steam (`--no-steam`, `--steam-api PATH`, or `BB_NO_STEAM`,
/// `BB_STEAM_API`).
#[derive(Clone, Debug, Default)]
pub struct SteamOptions {
	/// Never load Steam.
	pub off: bool,
	/// The library to load instead of the one beside the executable (for development).
	pub library: Option<PathBuf>,
}

impl SteamOptions {
	/// The options the environment gives (`BB_NO_STEAM=1`, `BB_STEAM_API=PATH`); the
	/// command line's override them.
	pub fn from_env() -> SteamOptions {
		SteamOptions {
			off: std::env::var_os("BB_NO_STEAM").is_some_and(|v| v != "0"),
			library: std::env::var_os("BB_STEAM_API").map(PathBuf::from),
		}
	}
}

/// What [`decide`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
	/// Not a Steam install, or Steam turned off: never load Steam.
	Off(String),
	/// A Steam install: load the library at this path.
	Load(PathBuf),
}

/// Whether the install at `data_dir` is Steam's and which library to load, without
/// loading anything: the install's rule ([`bb_steam::install`]) first, then the
/// player's switch, then the library (`options.library`, else beside the executable).
pub fn decide(data_dir: &Path, options: &SteamOptions, exe_dir: Option<&Path>) -> Decision {
	if options.off {
		return Decision::Off("turned off (--no-steam)".into());
	}
	if let Err(why) = bb_steam::install::steam_install(data_dir) {
		return Decision::Off(why);
	}
	let library = options.library.clone().or_else(|| exe_dir.map(|d| d.join(bb_steam::LIBRARY_NAME)));
	match library {
		Some(path) if path.is_file() => Decision::Load(path),
		Some(path) => Decision::Off(format!("no {} at {}", bb_steam::LIBRARY_NAME, path.display())),
		None => Decision::Off("no folder for the executable".into()),
	}
}

/// Whether the install at `data_dir` is Steam's: the scripts' `isSteamBuild()` (the
/// original's GOG build links to GOG's store pages where Steam's links to Steam's).
pub fn is_steam_install(data_dir: &Path) -> bool {
	bb_steam::install::steam_install(data_dir).is_ok()
}

/// Steam's API started, as the game's [`UserStats`].
pub struct SteamStats(Steam);

impl SteamStats {
	/// Dispatch Steam's callbacks (once a frame).
	pub fn run_callbacks(&self) {
		self.0.run_callbacks();
	}
}

impl UserStats for SteamStats {
	fn achievement(&self, id: &str) -> Option<bool> {
		self.0.achievement(id)
	}
	fn set_achievement(&self, id: &str) -> bool {
		let set = self.0.set_achievement(id);
		eprintln!("steam: achievement {id}{}", if set { "" } else { " (Steam refused it)" });
		set
	}
	fn stat(&self, name: &str) -> Option<i32> {
		self.0.stat(name)
	}
	fn set_stat(&self, name: &str, value: i32) -> bool {
		self.0.set_stat(name, value)
	}
	fn store(&self) -> bool {
		self.0.store()
	}
}

/// Decide, and on a Steam install start Steam with `load`; `None` (achievements off)
/// otherwise, said once in the log. `load` is called only for a Steam install with
/// Steam not turned off.
pub fn start_with<T>(data_dir: &Path, options: &SteamOptions, exe_dir: Option<&Path>, load: impl FnOnce(&Path) -> Result<T, String>) -> Option<T> {
	match decide(data_dir, options, exe_dir) {
		Decision::Off(why) => {
			eprintln!("steam: achievements off: {why}");
			None
		}
		Decision::Load(path) => match load(&path) {
			Ok(steam) => {
				eprintln!("steam: achievements on ({})", path.display());
				Some(steam)
			}
			Err(why) => {
				eprintln!("steam: achievements off: {why}");
				None
			}
		},
	}
}

/// [`start_with`] Steam's real API.
///
/// # Safety
/// Call before any other thread may read or write the environment (see
/// [`Steam::start`]).
pub unsafe fn start(data_dir: &Path, options: &SteamOptions) -> Option<Rc<SteamStats>> {
	let exe = std::env::current_exe().ok();
	let exe_dir = exe.as_deref().and_then(Path::parent);
	// SAFETY: the caller's guarantee.
	start_with(data_dir, options, exe_dir, |path| unsafe { Steam::start(path, bb_steam::APP_ID) }).map(|s| Rc::new(SteamStats(s)))
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::fs;

	fn scratch(name: &str) -> PathBuf {
		let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/tmp/bb-app-steam").join(name);
		let _ = fs::remove_dir_all(&dir);
		fs::create_dir_all(&dir).unwrap();
		dir
	}

	/// A GOG-style install and a Steam-style one, each with a (fake) library beside a
	/// fake executable.
	fn installs(name: &str) -> (PathBuf, PathBuf, PathBuf) {
		let root = scratch(name);
		let gog = root.join("GOG Games/Battle Brothers/data");
		let steam = root.join("steamapps/common/Battle Brothers/data");
		fs::create_dir_all(&gog).unwrap();
		fs::create_dir_all(&steam).unwrap();
		fs::write(root.join("steamapps/appmanifest_365360.acf"), "\"AppState\"\n{\n\t\"installdir\"\t\t\"Battle Brothers\"\n}\n").unwrap();
		let exe_dir = root.join("bb");
		fs::create_dir_all(&exe_dir).unwrap();
		fs::write(exe_dir.join(bb_steam::LIBRARY_NAME), b"not loaded").unwrap();
		(gog, steam, exe_dir)
	}

	/// A non-Steam install never touches Steam: the loader isn't called, even with a
	/// library beside bb and one named explicitly.
	#[test]
	fn a_non_steam_install_never_loads_steam() {
		let (gog, _, exe_dir) = installs("gog");
		let options = SteamOptions { off: false, library: Some(exe_dir.join(bb_steam::LIBRARY_NAME)) };
		let started = start_with(&gog, &options, Some(&exe_dir), |_| -> Result<(), String> { panic!("Steam loaded for a GOG install") });
		assert!(started.is_none());
		assert!(!is_steam_install(&gog));
	}

	#[test]
	fn no_steam_turns_it_off_on_a_steam_install() {
		let (_, steam, exe_dir) = installs("off");
		let options = SteamOptions { off: true, library: None };
		let started = start_with(&steam, &options, Some(&exe_dir), |_| -> Result<(), String> { panic!("Steam loaded with --no-steam") });
		assert!(started.is_none());
		assert!(is_steam_install(&steam));
	}

	#[test]
	fn a_steam_install_loads_the_library_beside_bb_or_the_named_one() {
		let (_, steam, exe_dir) = installs("on");
		let beside = exe_dir.join(bb_steam::LIBRARY_NAME);
		assert_eq!(decide(&steam, &SteamOptions::default(), Some(&exe_dir)), Decision::Load(beside.clone()));
		let named = SteamOptions { off: false, library: Some(beside.clone()) };
		assert_eq!(decide(&steam, &named, None), Decision::Load(beside));
		// The loader's failure (Steam not running, say) is achievements off, not an error.
		assert!(start_with(&steam, &SteamOptions::default(), Some(&exe_dir), |_| -> Result<(), String> { Err("Steam is not running".into()) }).is_none());
	}

	#[test]
	fn a_steam_install_without_the_library_is_off() {
		let (_, steam, exe_dir) = installs("missing");
		fs::remove_file(exe_dir.join(bb_steam::LIBRARY_NAME)).unwrap();
		assert!(matches!(decide(&steam, &SteamOptions::default(), Some(&exe_dir)), Decision::Off(why) if why.contains(bb_steam::LIBRARY_NAME)));
	}
}
