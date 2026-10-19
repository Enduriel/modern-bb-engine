//! Whether an install of the game is Steam's: the rule that decides if bb loads Steam's
//! library at all.
//!
//! **The rule:** the data folder bb runs (`<game>/data`) sits in a Steam library, as
//! `<library>/steamapps/common/<installdir>/data`, and that library's
//! `steamapps/appmanifest_365360.acf` names `<installdir>` as its install folder. Steam
//! writes that manifest for every game it installs, and only Steam does, so a GOG or
//! other copy (even one copied out of a Steam library) is never taken for Steam's. The
//! folder names are compared ignoring ASCII case (Steam's own older `SteamApps`, and
//! case-insensitive file systems). Paths are resolved first (symlinks such as
//! `~/.steam/steam`). The player can turn Steam off regardless (`bb --no-steam`).

use std::fs;
use std::path::{Path, PathBuf};

/// A Steam install of the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteamInstall {
	/// The library's `steamapps` folder.
	pub steamapps: PathBuf,
	/// The game's folder (`steamapps/common/<installdir>`).
	pub game_dir: PathBuf,
}

/// The manifest Steam keeps for the game in its library.
pub fn manifest_name() -> String {
	format!("appmanifest_{}.acf", crate::APP_ID)
}

/// The Steam install the data folder `data_dir` belongs to, or why it isn't one (said
/// once in bb's log).
pub fn steam_install(data_dir: &Path) -> Result<SteamInstall, String> {
	let data_dir = fs::canonicalize(data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;
	let not = |why: &str| Err(format!("{} is not in a Steam library ({why})", data_dir.display()));
	let Some(game_dir) = data_dir.parent() else { return not("no game folder above it") };
	let Some(common) = game_dir.parent().filter(|d| named(d, "common")) else {
		return not("its game folder is not in a `common` folder");
	};
	let Some(steamapps) = common.parent().filter(|d| named(d, "steamapps")) else {
		return not("no `steamapps` folder above `common`");
	};
	let manifest = steamapps.join(manifest_name());
	let Ok(text) = fs::read_to_string(&manifest) else {
		return not(&format!("no {}", manifest_name()));
	};
	let Some(install_dir) = acf_value(&text, "installdir") else {
		return not(&format!("{} names no installdir", manifest_name()));
	};
	if !game_dir.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&install_dir)) {
		return not(&format!("{} installs the game in `{install_dir}`", manifest_name()));
	}
	Ok(SteamInstall { steamapps: steamapps.to_path_buf(), game_dir: game_dir.to_path_buf() })
}

/// Whether the folder's own name is `name`, ignoring ASCII case.
fn named(dir: &Path, name: &str) -> bool {
	dir.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
}

/// The value of the first `"key"  "value"` line in a Valve KeyValues (`.acf`) text.
fn acf_value(text: &str, key: &str) -> Option<String> {
	text.lines().find_map(|line| {
		let mut quoted = line.split('"').skip(1).step_by(2);
		let k = quoted.next()?;
		let v = quoted.next()?;
		k.eq_ignore_ascii_case(key).then(|| v.to_string())
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn acf_values_are_read_by_key() {
		let manifest = "\"AppState\"\n{\n\t\"appid\"\t\t\"365360\"\n\t\"installdir\"\t\t\"Battle Brothers\"\n}\n";
		assert_eq!(acf_value(manifest, "installdir").as_deref(), Some("Battle Brothers"));
		assert_eq!(acf_value(manifest, "appid").as_deref(), Some("365360"));
		assert_eq!(acf_value(manifest, "missing"), None);
	}
}
