//! Battle Brothers on the modern engine, as a game: the scripts booted into a campaign
//! ([`game::Game`]), with their UI ([`front`]) and the player's keys ([`keys`]). The `bb`
//! binary puts it in a window.

pub mod clipboard;
pub mod display;
#[cfg(feature = "ui")]
pub mod frame_scenes;
pub mod fps;
pub mod front;
pub mod game;
pub mod game_cursor;
pub mod heap_profile;
pub mod keys;
pub mod logs;
pub mod pacing;
pub mod platform;
pub mod point_modes;
pub mod procedural;
pub mod requirements;
pub mod screen_scale;
pub mod script_profile;
pub mod settings;
pub mod shown;
pub mod stats;
pub mod steam;
pub mod system_cursor;
pub mod wheel;
pub mod world_scale;

/// What `bb --version` prints: the version, the Rust target, and whether the build has
/// the UI (`bb 0.1.0 (x86_64-unknown-linux-gnu, ui)`).
pub fn version() -> String {
	let ui = if cfg!(feature = "ui") { ", ui" } else { "" };
	format!("bb {} ({}{ui})", env!("CARGO_PKG_VERSION"), env!("BB_TARGET"))
}
