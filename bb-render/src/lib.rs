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
//! - [`UiPage`]: the UI page's frame on the GPU, copied from the browser's paints.
//! - [`UiLayer`]: the UI page blended over a frame.
//! - [`Presenter`]: copies a finished frame onto a window's surface.
//! - [`portrait`]: an entity drawn on its own, for the UI's procedural images.
//! - [`cursor_image`]: the cursor sprites as system cursor pictures.
//! - [`frame_diff`]: where a frame of ours differs from a screenshot of the original.

mod cursor;
pub mod cursor_image;
mod fast_hash;
mod fog;
pub use fog::drift_time as fog_drift_time;
pub mod fonts;
pub mod frame_diff;
/// The frame's spans (moved to `squirrel_bb`, which the engine's frame times too).
pub use squirrel_bb::frame_timing;
mod gpu;
mod gpu_timing;
mod kept_map;
mod kept_sprites;
pub mod labels;
mod map_quad;
mod memory;
mod mips;
pub mod orientation;
mod pages;
pub mod portrait;
mod present;
pub mod residency;
mod screenshot;
pub mod sprite;
pub mod tactical;
mod target;
#[cfg(test)]
mod test_sprites;
mod text;
mod texture;
mod ui_layer;
mod ui_page;
mod usage;
mod water;
pub mod world;
mod world_pass;
mod world_items;
mod world_renderer;

pub use cursor::{frame_at as cursor_frame_at, frame_index_at as cursor_frame_index_at};
pub use gpu::Gpu;
pub use memory::{RendererMemory, TextureMemory};
pub use usage::AtlasUse;
pub use present::Presenter;
pub use screenshot::{save_png, write_png};
pub use target::{RenderTarget, TARGET_FORMAT};
pub use bb_assets::mips::mip_chain;
pub use texture::{GpuTexture, TEXTURE_FORMAT};
pub use ui_layer::UiLayer;
#[cfg(target_os = "linux")]
pub use ui_page::DmaBuf;
#[cfg(target_os = "macos")]
pub use ui_page::IOSurface;
#[cfg(windows)]
pub use ui_page::D3D11Texture;
pub use ui_page::{CopyStats as UiCopyStats, Region as UiRegion, UiPage};
pub use world_renderer::{Atlases, Clock, Composed, Discard, FrameStats, WorldMap, WorldRenderer};
