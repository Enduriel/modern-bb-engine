//! The Battle Brothers engine layer: the VFS (reading the game's `.dat` archives
//! + mod overlay), decrypting + loading the compiled scripts, and (later) the
//! game subsystems (World/Tactical/etc.) and host boot sequence.
//!
//! It sits above `squirrel` (the VM bindings) and `squirrel-bb` (the module
//! system + native globals).
//!
//! Current pieces:
//!   * [`assets`] — loading the sprite sheets (`.brush`) at startup
//!   * [`crypt`] — XXTEA, the cipher on the compiled scripts in the `.dat`s
//!   * [`files`] — the game's other files (textures, …), readable from any thread
//!   * [`vfs`]   — reading `.dat` (ZIP) archives
//!   * [`script_cache`] — transcoded scripts kept on disk for warm starts
//!   * [`log_files`] — bb's log files, the earlier sessions' kept as zips
//!   * [`harness`] — headless boot steps over a real install (menu, new campaign)
//!   * [`probe_aside`] — the installed mods while a probe run holds them aside
//!   * [`mod_corpus`] — the reference mod corpus the corpus tests mount, pinned by hash
//!   * [`heap`] — the C heap's usage, for the memory tests and tools
//!   * [`install`] — finding the player's copy of the game (overrides, the chosen folder, Steam)
//!   * [`script_profile`] — reports of where the scripts' time goes, by function and mod
//!
//! Next: the 3.0.4 → current-version bytecode transcoder (the game ships Squirrel
//! 3.0.4 / 32-bit compiled bytecode; we run a newer 64-bit VM, so the `.cnut`
//! bytecode is converted on load rather than matched VM-side).

pub mod assets;
pub mod crypt;
pub mod datsource;
pub mod files;
pub mod game_queue;
pub mod harness;
pub mod heap;
pub mod install;
pub mod load_order;
pub mod log_files;
pub mod mod_corpus;
pub mod mounts;
pub mod probe_aside;
pub mod script_cache;
pub mod script_profile;
pub mod tactical_player;
#[cfg(test)]
mod test_game;
pub mod transcode;
pub mod vfs;

pub use datsource::DatScriptSource;
pub use script_cache::ScriptCache;
pub use transcode::transcode_cnut;
pub use vfs::DatArchive;
