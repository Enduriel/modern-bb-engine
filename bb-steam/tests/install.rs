//! The rule for a Steam install ([`bb_steam::install`]) on folders laid out as Steam's
//! libraries and GOG's are.

use std::fs;
use std::path::{Path, PathBuf};

use bb_steam::install::{manifest_name, steam_install};

/// A fresh folder for one test, under the target folder (never /tmp).
fn scratch(name: &str) -> PathBuf {
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bb-steam-install").join(name);
	let _ = fs::remove_dir_all(&dir);
	fs::create_dir_all(&dir).unwrap();
	dir
}

const MANIFEST: &str = "\"AppState\"\n{\n\t\"appid\"\t\t\"365360\"\n\t\"installdir\"\t\t\"Battle Brothers\"\n}\n";

fn library(name: &str, manifest: Option<&str>) -> PathBuf {
	let root = scratch(name);
	let data = root.join("steamapps/common/Battle Brothers/data");
	fs::create_dir_all(&data).unwrap();
	if let Some(text) = manifest {
		fs::write(root.join("steamapps").join(manifest_name()), text).unwrap();
	}
	data
}

#[test]
fn a_steam_library_with_the_manifest_is_steam() {
	let data = library("steam", Some(MANIFEST));
	let install = steam_install(&data).expect("a Steam install");
	assert!(install.steamapps.ends_with("steamapps"));
	assert!(install.game_dir.ends_with("Battle Brothers"));
}

#[test]
fn without_the_manifest_it_is_not() {
	let data = library("no_manifest", None);
	assert!(steam_install(&data).unwrap_err().contains("appmanifest_365360.acf"));
}

#[test]
fn a_manifest_for_another_folder_is_not_this_install() {
	let data = library("other_dir", Some(&MANIFEST.replace("Battle Brothers", "Battle Brothers Beta")));
	assert!(steam_install(&data).is_err());
}

/// A macOS app bundle's data, in the game's folder in the library, is the install's.
#[test]
fn a_mac_app_bundles_data_is_steam() {
	let data = library("mac", Some(MANIFEST));
	let bundled = data.parent().unwrap().join("Battle Brothers.app/Contents/Resources/data");
	fs::create_dir_all(&bundled).unwrap();
	let install = steam_install(&bundled).expect("a Steam install");
	assert!(install.game_dir.ends_with("common/Battle Brothers"), "{}", install.game_dir.display());
}

/// A GOG install (no steamapps/common around it) is never Steam's.
#[test]
fn a_gog_install_is_not() {
	let root = scratch("gog");
	let data = root.join("GOG Games/Battle Brothers/data");
	fs::create_dir_all(&data).unwrap();
	assert!(steam_install(&data).is_err());
}

/// `what` is missing: fails the test unless `BB_ALLOW_SKIPS=1` lets it skip (as
/// `bb_engine::harness::skip`, which this crate can't use).
fn skip(what: &str) {
	if std::env::var_os("BB_ALLOW_SKIPS").is_some_and(|v| v == "1") {
		eprintln!("skipping: {what}");
	} else {
		panic!("this test can't run here: {what} (set BB_ALLOW_SKIPS=1 to skip it on a machine without it)");
	}
}

/// The real Steam install on this host, as `steamlocate` finds it (read-only: only its
/// folders and manifest are looked at).
#[test]
fn the_real_steam_install_is_steam() {
	let found = steamlocate::SteamDir::locate().ok().and_then(|steam| {
		let (game, library) = steam.find_app(365_360).ok()??;
		Some(library.resolve_app_dir(&game).join("data"))
	});
	let Some(data) = found.filter(|data| data.is_dir()) else {
		skip("no Steam install of the game on this machine");
		return;
	};
	steam_install(&data).expect("the Steam install");
}
