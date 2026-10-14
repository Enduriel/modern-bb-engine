//! [`WorldRenderer`]: draws the world map frame by frame, loading atlases on demand.
//!
//! No preload list is read. An atlas starts decoding on a
//! worker thread when a script hints at it (a brush set on a sprite, tile or detail,
//! or `Tactical.addResource`; see `squirrel_bb::Sprites::take_hints`) or when a frame
//! first needs it. A live frame skips sprites whose atlas isn't ready yet, and they
//! appear once it is; a frame for a screenshot waits for every atlas instead.

use std::sync::Arc;

use bb_assets::{BitmapFont, FileSource, Image, ImageLoader, Slot, SpriteRegistry, TextureCache};
use squirrel_bb::FogPatch;
use squirrel_bb::scene::WorldScene;

use crate::fog::FogPass;
use crate::gpu::Gpu;
use crate::labels::{LabelFont, WHITE};
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
	/// The label font and its atlas's path (`None` if the data lacks it).
	label_font: Option<(BitmapFont, String)>,
	/// 1×1 white, for solid fills (label plates).
	white: GpuTexture,
	fog: FogPass,
	/// Seconds of real time, for the fog's drift.
	real_time: f32,
}

/// The fog of war's texture.
const FOG_TEXTURE: &str = "gfx/fow.png";

/// The font world labels are drawn in.
const LABEL_FONT: &str = "gfx/fonts/cinzel_bold_20";

impl WorldRenderer {
	/// A renderer reading atlases from `files`, decoding on `threads` workers.
	pub fn new(gpu: &Gpu, files: Arc<dyn FileSource>, threads: usize) -> Self {
		let label_font = files
			.read(&format!("{LABEL_FONT}.fnt"))
			.and_then(|bytes| BitmapFont::parse(&bytes).ok())
			.map(|font| (font, format!("{LABEL_FONT}.png")));
		let white = Image {
			width: 1,
			height: 1,
			rgba: vec![255; 4],
		};
		WorldRenderer {
			sprites: SpriteRenderer::new(gpu),
			textures: TextureCache::new(ImageLoader::new(files, threads)),
			label_font,
			white: GpuTexture::upload(gpu, "white", &white),
			fog: FogPass::new(gpu),
			real_time: 0.0,
		}
	}

	/// Take a change to the fog of war's mask (from `WorldGrid::take_fog_patch`).
	pub fn update_fog(&mut self, gpu: &Gpu, patch: &FogPatch) {
		self.fog.update_mask(gpu, patch);
	}

	/// The real time in seconds (`Time.getRealTimeF()`): the fog drifts with it.
	pub fn set_real_time(&mut self, seconds: f32) {
		self.real_time = seconds;
	}

	/// Start loading `paths` (the scripts' hints) in the background. Only images are
	/// loaded here; other hinted resources, such as sounds, are left to their own
	/// subsystem.
	pub fn prefetch(&mut self, paths: impl IntoIterator<Item = String>) {
		for path in paths.into_iter().filter(|p| p.ends_with(".png")) {
			self.textures.prefetch(&path);
		}
	}

	/// Draw `scene` into `target`, clearing it first: the map, then the fog of war
	/// where its mask has it (once [`update_fog`](Self::update_fog) gave one), then the
	/// labels.
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
		let font = self
			.label_font
			.as_ref()
			.map(|(font, texture)| LabelFont { font, texture });
		let world = world_quads(scene, registry, view, font);
		let mut needed: Vec<&str> = world
			.atlases
			.paths
			.iter()
			.map(String::as_str)
			.filter(|p| *p != WHITE)
			.collect();
		if scene.fog.on {
			needed.push(FOG_TEXTURE);
		}
		for path in needed {
			match atlases {
				Atlases::WaitForAll => {
					let _ = self.textures.get_now(path, upload);
				}
				Atlases::DrawReady => {
					let _ = self.textures.get(path);
				}
			}
		}

		// The frame's atlases, renumbered; `None` for one not usable.
		let mut stats = FrameStats::default();
		let mut ready: Vec<&GpuTexture> = Vec::new();
		let slots: Vec<Option<usize>> = world
			.atlases
			.paths
			.iter()
			.map(|path| match self.textures.slot(path) {
				_ if path == WHITE => {
					ready.push(&self.white);
					Some(ready.len() - 1)
				}
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
		let mut usable = |quads: &[Quad]| -> Vec<Quad> {
			quads
				.iter()
				.filter_map(|quad| {
					let slot = slots[quad.texture];
					if slot.is_none() {
						stats.waiting += 1;
					}
					slot.map(|texture| Quad { texture, ..*quad })
				})
				.collect()
		};
		let map = usable(&world.quads[..world.labels_from]);
		let labels = usable(&world.quads[world.labels_from..]);
		stats.quads = map.len() + labels.len();
		let uniforms = view.uniforms();
		self.sprites.draw(
			gpu,
			target,
			&uniforms,
			Some([0.0, 0.0, 0.0, 1.0]),
			&map,
			&ready,
		);

		if scene.fog.on {
			if let Some(Slot::Ready(fog_texture)) = self.textures.slot(FOG_TEXTURE) {
				self.fog.draw(
					gpu,
					target,
					view,
					fog_texture,
					scene.fog.extent,
					self.real_time,
				);
			}
		}

		self.sprites
			.draw(gpu, target, &uniforms, None, &labels, &ready);
		stats
	}
}
