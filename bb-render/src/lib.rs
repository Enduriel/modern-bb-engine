//! The renderer, on wgpu.
//!
//! Frames render into offscreen targets: tests read them back as images to compare
//! with screenshots of the original, and a window presents them.
//!
//! - [`Gpu`]: the device, headless or for a window.
//! - [`RenderTarget`]: an offscreen colour target with readback.
//! - [`GpuTexture`]: an uploaded atlas with its mip chain.
//! - [`sprite`]: the original's sprite program and the quad batcher.
//! - [`world`]: the world map, from the game's [`WorldScene`](squirrel_bb::scene::WorldScene).
//! - [`WorldRenderer`]: draws it frame by frame, loading atlases on demand.
//! - [`UiLayer`]: the UI page blended over a frame.
//! - [`Presenter`]: copies a finished frame onto a window's surface.
//! - [`portrait`]: an entity drawn on its own, for the UI's procedural images.
//! - [`frame_diff`]: where a frame of ours differs from a screenshot of the original.

mod cursor;
mod fast_hash;
mod fog;
pub mod fonts;
pub mod frame_diff;
pub mod frame_timing;
mod gpu;
mod kept_map;
pub mod labels;
mod memory;
mod mips;
mod pages;
pub mod portrait;
mod present;
pub mod residency;
mod screenshot;
pub mod sprite;
pub mod tactical;
mod tactical_clip;
mod target;
#[cfg(test)]
mod test_sprites;
mod text;
mod texture;
mod ui_layer;
mod usage;
mod water;
pub mod world;
mod world_pass;
mod world_renderer;

pub use cursor::frame_at as cursor_frame_at;
pub use gpu::Gpu;
pub use memory::{RendererMemory, TextureMemory};
pub use usage::AtlasUse;
pub use present::Presenter;
pub use screenshot::{save_png, write_png};
pub use target::{RenderTarget, TARGET_FORMAT};
pub use bb_assets::mips::mip_chain;
pub use texture::{GpuTexture, TEXTURE_FORMAT};
pub use ui_layer::{Region as UiRegion, UiLayer, UploadStats};
pub use world_renderer::{Atlases, Clock, Discard, FrameStats, WorldMap, WorldRenderer};
