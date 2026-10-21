//! The binding against Steam's real library: it loads and exports every function the
//! binding calls. Steam's API is never started here (no `SteamAPI_InitFlat`), so the
//! test can't touch the player's Steam account. Opt-in (ignored): run it with
//! `--include-ignored` and `BB_STEAM_API` naming the library (e.g. the one `bb-package`
//! fetched and checked).

use std::path::PathBuf;

#[test]
#[ignore = "opt-in: needs BB_STEAM_API naming Steam's library"]
fn the_pinned_library_exports_the_binding() {
	let path = std::env::var_os("BB_STEAM_API").map(PathBuf::from).expect("BB_STEAM_API naming Steam's library");
	if let Some(pinned) = bb_steam::pinned::library_for(env!("BB_STEAM_TARGET")) {
		let hash = bb_steam::sha256::file_hex(&path).expect("the library reads");
		bb_steam::pinned::verify(&path.display().to_string(), &hash, pinned.sha256).expect("the pinned library");
	}
	bb_steam::client::check_library(&path).expect("every function the binding calls");
}
