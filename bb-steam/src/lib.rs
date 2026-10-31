//! Steam for the windowed game: achievements and stats, as the original's.
//!
//! - [`install`] decides whether an install is Steam's (the rule is there). Only then
//!   does bb load Steam's library at all: a GOG or other install never touches it.
//! - [`library`] loads a shared library at run time and resolves its symbols (the
//!   platform layer: `dlopen` on Unix, `LoadLibraryW` on Windows).
//! - [`client`] is the binding: the few calls of Steam's flat C API the game needs
//!   (`SteamAPI_InitFlat`, `ISteamUserStats`, `SteamAPI_RunCallbacks`, `SteamAPI_Shutdown`).
//! - [`pinned`] names the one copy of Steam's redistributable library a release package
//!   carries, with the hashes the packaging checks it against (`bb-sha256` computes them).
//!
//! The game's natives don't know Steam: `squirrel-bb` calls a `UserStats` service, which
//! `bb-app` makes from a [`client::Steam`].

pub mod client;
pub mod install;
pub mod library;
pub mod pinned;

/// Battle Brothers' Steam app id.
pub const APP_ID: u32 = 365_360;

/// The file name of Steam's library on this system, which a package carries beside the
/// executable.
#[cfg(target_os = "windows")]
pub const LIBRARY_NAME: &str = "steam_api64.dll";
#[cfg(target_os = "macos")]
pub const LIBRARY_NAME: &str = "libsteam_api.dylib";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub const LIBRARY_NAME: &str = "libsteam_api.so";
