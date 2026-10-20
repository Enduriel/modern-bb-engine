//! Battle Brothers on the modern engine, as a game: the scripts booted into a campaign
//! ([`game::Game`]), with their UI ([`front`]) and the player's keys ([`keys`]). The `bb`
//! binary puts it in a window.

pub mod display;
#[cfg(feature = "ui")]
pub mod frame_scenes;
pub mod front;
pub mod game;
pub mod keys;
pub mod logs;
pub mod platform;
pub mod settings;
pub mod stats;
pub mod steam;
pub mod wheel;
