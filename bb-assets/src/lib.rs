//! The game's asset formats, with no VM, file system or renderer attached.
//!
//! Both the headless game logic (`squirrel-bb`, which answers `doesBrushExist`) and
//! the renderer read these, so they live below both.
//!
//! - [`brush`]: `.brush` sprite sheets, parsed exactly as the original reads them.
//! - [`SpriteRegistry`]: every loaded sprite by name, with the original's
//!   first-one-wins rule for duplicate names.
//! - [`brush_load_order`]: which brush files load at startup, in the original's
//!   order.
//! - [`name_hash`]: the original's 64-bit string hash that keys sprites.

pub mod brush;
mod hash;
mod load_order;
mod reader;
mod registry;

pub use brush::{BrushFrame, BrushSheet, BrushSprite, SheetHeader};
pub use hash::name_hash;
pub use load_order::{BRUSH_DIR, brush_load_order};
pub use registry::{LoadedSheet, SpriteId, SpriteRegistry};
