//! The game's sound through kira.
//!
//! The game's side (`squirrel_bb`'s sound manager) decides what plays, how loud and
//! when, as the original does; this crate plays it:
//! - [`KiraOut`]: the [`AudioOut`](squirrel_bb::AudioOut) the window uses, on the
//!   device (or kira's mock backend in tests);
//! - [`gains`]: the original's library and driver's volume, pan and distance rules;
//! - [`pan`]: positional sounds panned by their place on the screen (a deliberate
//!   divergence; the original's pan is kept in [`gains::positional`]);
//! - [`SoundBank`]: sounds read from the game's files on first use, short ones decoded
//!   once and kept, long ones streamed.

mod bank;
pub mod gains;
mod kira_out;
pub mod pan;

pub use bank::{Sound, SoundBank};
pub use kira::backend::mock::MockBackend;
pub use kira_out::{DEVICE_BUFFER_FRAMES, DeviceStats, KiraOut};
