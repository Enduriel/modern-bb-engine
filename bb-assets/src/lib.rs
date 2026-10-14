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
//! - [`BitmapFont`]: the engine's `.fnt` bitmap fonts (world labels).
//! - With the `images` feature: [`decode_png`] and on-demand loading on worker
//!   threads ([`TextureCache`]), which replaces vanilla's preload lists.

pub mod brush;
mod font;
mod hash;
#[cfg(feature = "images")]
mod image;
mod load_order;
#[cfg(feature = "images")]
mod loader;
mod reader;
mod registry;

pub use brush::{BrushFrame, BrushSheet, BrushSprite, SheetHeader};
pub use font::{BitmapFont, GlyphUv};
pub use hash::name_hash;
#[cfg(feature = "images")]
pub use image::{Image, decode_png};
pub use load_order::{BRUSH_DIR, brush_load_order};
#[cfg(feature = "images")]
pub use loader::{Decoded, FileSource, ImageLoader, Slot, TextureCache, load_image};
pub use registry::{LoadedSheet, SpriteId, SpriteRegistry};
