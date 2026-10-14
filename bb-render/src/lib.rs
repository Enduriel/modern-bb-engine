//! The renderer, on wgpu.
//!
//! It starts headless: frames render into offscreen targets that are read back as
//! images, so tests can compare them with screenshots of the original. A window comes
//! later and reuses the same pieces.
//!
//! - [`Gpu`]: the device, opened without a window.
//! - [`RenderTarget`]: an offscreen colour target with readback.
//! - [`GpuTexture`]: an uploaded atlas with its mip chain.
//! - [`sprite`]: the original's sprite program and the quad batcher.
//! - [`world`]: the world map, from the game's [`WorldScene`](squirrel_bb::scene::WorldScene).
//! - [`WorldRenderer`]: draws it frame by frame, loading atlases on demand.

mod gpu;
mod screenshot;
pub mod sprite;
mod target;
mod texture;
pub mod world;
mod world_renderer;

pub use gpu::Gpu;
pub use screenshot::{save_png, write_png};
pub use target::{RenderTarget, TARGET_FORMAT};
pub use texture::{GpuTexture, TEXTURE_FORMAT, mip_chain};
pub use world_renderer::{Atlases, FrameStats, WorldRenderer};
