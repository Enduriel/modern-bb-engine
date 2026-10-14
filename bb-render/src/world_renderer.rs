//! [`WorldRenderer`]: draws the world map frame by frame, loading atlases on demand.
//!
//! No preload list is read. An atlas starts decoding on a
//! worker thread when a script hints at it (a brush set on a sprite, tile or detail,
//! or `Tactical.addResource`; see `squirrel_bb::Sprites::take_hints`) or when a frame
//! first needs it. A live frame skips sprites whose atlas isn't ready yet, and they
//! appear once it is; a frame for a screenshot waits for every atlas instead.

use std::sync::Arc;

use bb_assets::{FileSource, ImageLoader, Slot, SpriteRegistry, TextureCache};
use squirrel_bb::scene::WorldScene;

use crate::gpu::Gpu;
use crate::sprite::{Quad, SpriteRenderer};
use crate::target::RenderTarget;
use crate::texture::GpuTexture;
use crate::world::{WorldView, world_quads};

/// Whether a frame waits for atlases that are still loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atlases {
	/// Skip sprites whose atlas isn't loaded yet (a live frame).
	DrawReady,
	/// Load every atlas the frame needs first (screenshots, tests).
	WaitForAll,
}

/// What one frame drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
	pub quads: usize,
	/// Quads skipped because their atlas was still loading.
	pub waiting: usize,
	/// Atlases that failed to load (their quads are skipped).
	pub failed: usize,
}

/// The world map renderer and its atlas cache.
pub struct WorldRenderer {
	sprites: SpriteRenderer,
	textures: TextureCache<GpuTexture>,
}

impl WorldRenderer {
	/// A renderer reading atlases from `files`, decoding on `threads` workers.
	pub fn new(gpu: &Gpu, files: Arc<dyn FileSource>, threads: usize) -> Self {
		WorldRenderer {
			sprites: SpriteRenderer::new(gpu),
			textures: TextureCache::new(ImageLoader::new(files, threads)),
		}
	}

	/// Start loading `paths` (the scripts' hints) in the background. Only images are
	/// loaded here; other hinted resources, such as sounds, are left to their own
	/// subsystem.
	pub fn prefetch(&mut self, paths: impl IntoIterator<Item = String>) {
		for path in paths.into_iter().filter(|p| p.ends_with(".png")) {
			self.textures.prefetch(&path);
		}
	}

	/// Draw `scene` into `target`, clearing it first.
	pub fn draw(
		&mut self,
		gpu: &Gpu,
		target: &RenderTarget,
		scene: &WorldScene,
		registry: &SpriteRegistry,
		view: &WorldView,
		atlases: Atlases,
	) -> FrameStats {
		let upload = |path: &str, image: bb_assets::Image| GpuTexture::upload(gpu, path, &image);
		self.textures.pump(upload);
		let (quads, list) = world_quads(scene, registry, view);
		for path in &list.paths {
			match atlases {
				Atlases::WaitForAll => {
					let _ = self.textures.get_now(path, upload);
				}
				Atlases::DrawReady => {
					let _ = self.textures.get(path);
				}
			}
		}

		// The atlases this frame can use, renumbered; `None` for one not usable.
		let mut stats = FrameStats::default();
		let mut ready: Vec<&GpuTexture> = Vec::new();
		let slots: Vec<Option<usize>> = list
			.paths
			.iter()
			.map(|path| match self.textures.slot(path) {
				Some(Slot::Ready(texture)) => {
					ready.push(texture);
					Some(ready.len() - 1)
				}
				Some(Slot::Failed(_)) => {
					stats.failed += 1;
					None
				}
				_ => None,
			})
			.collect();
		let drawn: Vec<Quad> = quads
			.iter()
			.filter_map(|quad| {
				let slot = slots[quad.texture];
				if slot.is_none() {
					stats.waiting += 1;
				}
				slot.map(|texture| Quad { texture, ..*quad })
			})
			.collect();
		stats.quads = drawn.len();
		self.sprites.draw(
			gpu,
			target,
			&view.uniforms(),
			Some([0.0, 0.0, 0.0, 1.0]),
			&drawn,
			&ready,
		);
		stats
	}
}
