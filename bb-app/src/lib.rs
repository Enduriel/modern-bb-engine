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
pub mod procedural;
pub mod requirements;
pub mod script_profile;
pub mod settings;
pub mod shown;
pub mod stats;
pub mod steam;
pub mod system_cursor;
pub mod wheel;
