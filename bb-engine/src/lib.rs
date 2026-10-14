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
//!   * [`harness`] — headless boot steps over a real install (menu, new campaign)
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
pub mod mounts;
pub mod script_cache;
pub mod tactical_player;
pub mod transcode;
pub mod vfs;

pub use datsource::DatScriptSource;
pub use script_cache::ScriptCache;
pub use transcode::transcode_cnut;
pub use vfs::DatArchive;
