//! The binding against Steam's real library: it loads and exports every function the
//! binding calls. Steam's API is never started here (no `SteamAPI_InitFlat`), so the
//! test can't touch the player's Steam account. Runs with `BB_STEAM_API` naming the
//! library (e.g. the one `bb-package` fetched and checked); skips without it.

use std::path::PathBuf;

#[test]
fn the_pinned_library_exports_the_binding() {
	let Some(path) = std::env::var_os("BB_STEAM_API").map(PathBuf::from) else {
		eprintln!("skipped: set BB_STEAM_API to Steam's library");
		return;
	};
	if let Some(pinned) = bb_steam::pinned::library_for(env!("BB_STEAM_TARGET")) {
		let hash = bb_steam::sha256::file_hex(&path).expect("the library reads");
		bb_steam::pinned::verify(&path.display().to_string(), &hash, pinned.sha256).expect("the pinned library");
	}
	bb_steam::client::check_library(&path).expect("every function the binding calls");
}
