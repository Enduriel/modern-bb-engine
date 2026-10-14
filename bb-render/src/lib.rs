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
//! - [`Presenter`]: copies a finished frame onto a window's surface.

mod fog;
mod gpu;
pub mod labels;
mod present;
mod screenshot;
pub mod sprite;
pub mod tactical;
mod target;
mod text;
mod texture;
mod water;
pub mod world;
mod world_pass;
mod world_renderer;

pub use gpu::Gpu;
pub use present::Presenter;
pub use screenshot::{save_png, write_png};
pub use target::{RenderTarget, TARGET_FORMAT};
pub use texture::{GpuTexture, TEXTURE_FORMAT, mip_chain};
pub use world_renderer::{Atlases, Clock, FrameStats, WorldRenderer};
