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
use squirrel_bb::scene::tactical::TacticalScene;

use crate::fog::FogPass;
use crate::gpu::Gpu;
use crate::labels::{LabelFont, WHITE};
use crate::sprite::{Quad, SceneUniforms, SpriteRenderer};
use crate::tactical::tactical_quads;
use crate::target::RenderTarget;
use crate::texture::GpuTexture;
use crate::water::WaterPass;
use crate::world::{WorldFonts, WorldView, world_quads};

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
	/// The label and region-name fonts and their atlases' paths (`None` if the data
	/// lacks one).
	label_font: Option<(BitmapFont, String)>,
	region_font: Option<(BitmapFont, String)>,
	/// 1×1 white, for solid fills (label plates).
	white: GpuTexture,
	fog: FogPass,
	water: WaterPass,
	/// Seconds of real and virtual time, for the fog's and the water's drift.
	clock: Clock,
}

/// The game's clocks, in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clock {
	/// `Time.getRealTimeF()`.
	pub real: f32,
	/// The virtual clock, which runs at the game's speed.
	pub virtual_time: f32,
}

/// The fog of war's texture.
const FOG_TEXTURE: &str = "gfx/fow.png";

/// The water's texture.
const WATER_TEXTURE: &str = "gfx/water.png";

/// What shows where nothing is drawn.
const CLEAR: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

/// Where the bitmap fonts are; their atlases have no mips (see [`GpuTexture`]).
const FONT_DIR: &str = "gfx/fonts/";

/// The font world labels are drawn in.
const LABEL_FONT: &str = "gfx/fonts/cinzel_bold_20";

/// The font region names are drawn in.
const REGION_FONT: &str = "gfx/fonts/cinzel_bold_100";

impl WorldRenderer {
	/// A renderer reading atlases from `files`, decoding on `threads` workers.
	pub fn new(gpu: &Gpu, files: Arc<dyn FileSource>, threads: usize) -> Self {
		let load_font = |path: &str| {
			files
				.read(&format!("{path}.fnt"))
				.and_then(|bytes| BitmapFont::parse(&bytes).ok())
				.map(|font| (font, format!("{path}.png")))
		};
		let label_font = load_font(LABEL_FONT);
		let region_font = load_font(REGION_FONT);
		let white = Image {
			width: 1,
			height: 1,
			rgba: vec![255; 4],
		};
		WorldRenderer {
			sprites: SpriteRenderer::new(gpu),
			textures: TextureCache::new(ImageLoader::new(files, threads)),
			label_font,
			region_font,
			white: GpuTexture::upload(gpu, "white", &white),
			fog: FogPass::new(gpu),
			water: WaterPass::new(gpu),
			clock: Clock::default(),
		}
	}

	/// Take a change to the fog of war's mask (from `WorldGrid::take_fog_patch`).
	pub fn update_fog(&mut self, gpu: &Gpu, patch: &FogPatch) {
		self.fog.update_mask(gpu, patch);
	}

	/// The game's clocks: the fog drifts with real time, the water with virtual time.
	pub fn set_clock(&mut self, clock: Clock) {
		self.clock = clock;
	}

	/// Start loading `paths` (the scripts' hints) in the background. Only images are
	/// loaded here; other hinted resources, such as sounds, are left to their own
	/// subsystem.
	pub fn prefetch(&mut self, paths: impl IntoIterator<Item = String>) {
		for path in paths.into_iter().filter(|p| p.ends_with(".png")) {
			self.textures.prefetch(&path);
		}
	}

	/// Draw `scene` into `target`, clearing it first: the water, the map, then the fog
	/// of war where its mask has it (once [`update_fog`](Self::update_fog) gave one),
	/// then the labels.
	pub fn draw(
		&mut self,
		gpu: &Gpu,
		target: &RenderTarget,
		scene: &WorldScene,
		registry: &SpriteRegistry,
		view: &WorldView,
		atlases: Atlases,
	) -> FrameStats {
		let upload = |path: &str, image: bb_assets::Image| {
			if path.starts_with(FONT_DIR) {
				GpuTexture::upload_flat(gpu, path, &image)
			} else {
				GpuTexture::upload(gpu, path, &image)
			}
		};
		self.textures.pump(upload);
		let fonts = WorldFonts {
			label: label_font(&self.label_font),
			region: label_font(&self.region_font),
		};
		let world = world_quads(scene, registry, view, fonts);
		let mut needed: Vec<&str> = world
			.atlases
			.paths
			.iter()
			.map(String::as_str)
			.filter(|p| *p != WHITE)
			.collect();
		needed.push(WATER_TEXTURE);
		if scene.fog_of_war {
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
		// The day's light: on lit sprites, the water and the fog.
		let ambient = argb_floats(scene.light.color);
		let uniforms = SceneUniforms {
			ambient_color: ambient,
			ambient_saturation: scene.light.saturation,
			..view.uniforms()
		};
		let water = match self.textures.slot(WATER_TEXTURE) {
			Some(Slot::Ready(water)) => self.water.draw(
				gpu,
				target,
				Some(CLEAR),
				view,
				water,
				scene.extent,
				self.clock.virtual_time,
				ambient,
			),
			_ => false,
		};
		let clear = (!water).then_some(CLEAR);
		self.sprites
			.draw(gpu, target, &uniforms, clear, &map, &ready);

		if scene.fog_of_war {
			if let Some(Slot::Ready(fog_texture)) = self.textures.slot(FOG_TEXTURE) {
				self.fog.draw(
					gpu,
					target,
					view,
					fog_texture,
					scene.extent,
					self.clock.real,
					ambient,
				);
			}
		}

		self.sprites
			.draw(gpu, target, &uniforms, None, &labels, &ready);
		stats
	}

	/// Draw a battle map's terrain into `target`, clearing it first (see [`crate::tactical`]).
	pub fn draw_tactical(
		&mut self,
		gpu: &Gpu,
		target: &RenderTarget,
		scene: &TacticalScene,
		registry: &SpriteRegistry,
		view: &WorldView,
		atlases: Atlases,
	) -> FrameStats {
		let map = tactical_quads(scene, registry, view);
		// The battle's light (the weather's `setAmbientLighting*`), and the depth of
		// field below the camera's level.
		let uniforms = SceneUniforms {
			ambient_color: argb_floats(scene.light.color),
			ambient_saturation: scene.light.saturation,
			camera_level: scene.camera.level.clamp(0, 3) as f32,
			dof_strength: crate::tactical::DOF_STRENGTH,
			..view.uniforms()
		};
		self.draw_quads(gpu, target, &map, &uniforms, CLEAR, atlases)
	}

	/// Entity `portrait` drawn on its own, without the sprites named in `hidden` (see
	/// [`crate::portrait`]); `None` if nothing in it can be drawn. Waits for its atlases.
	pub fn compose_portrait(
		&mut self,
		gpu: &Gpu,
		portrait: &squirrel_bb::scene::ScenePortrait,
		hidden: &[String],
		registry: &SpriteRegistry,
	) -> Result<Option<Image>, String> {
		let (width, height) = crate::portrait::portrait_size(portrait, hidden, registry);
		if width == 0 || height == 0 {
			return Ok(None);
		}
		let map = crate::portrait::portrait_quads(portrait, hidden, registry);
		let target = RenderTarget::new(gpu, width, height);
		let uniforms = SceneUniforms::centered([0.0, 0.0], width as f32, height as f32);
		self.draw_quads(gpu, &target, &map, &uniforms, [0.0; 4], Atlases::WaitForAll);
		target.read(gpu).map(Some)
	}

	/// Draw `map`'s quads into `target` over `clear`, with the atlases they use as ready
	/// (or waited for).
	fn draw_quads(
		&mut self,
		gpu: &Gpu,
		target: &RenderTarget,
		map: &crate::world::WorldQuads,
		uniforms: &SceneUniforms,
		clear: [f64; 4],
		atlases: Atlases,
	) -> FrameStats {
		let upload = |path: &str, image: bb_assets::Image| {
			if path.starts_with(FONT_DIR) {
				GpuTexture::upload_flat(gpu, path, &image)
			} else {
				GpuTexture::upload(gpu, path, &image)
			}
		};
		self.textures.pump(upload);
		for path in &map.atlases.paths {
			match atlases {
				Atlases::WaitForAll => {
					let _ = self.textures.get_now(path, upload);
				}
				Atlases::DrawReady => {
					let _ = self.textures.get(path);
				}
			}
		}
		let mut stats = FrameStats::default();
		let mut ready: Vec<&GpuTexture> = Vec::new();
		let slots: Vec<Option<usize>> = map
			.atlases
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
		let quads: Vec<Quad> = map
			.quads
			.iter()
			.filter_map(|quad| {
				let slot = slots[quad.texture];
				if slot.is_none() {
					stats.waiting += 1;
				}
				slot.map(|texture| Quad { texture, ..*quad })
			})
			.collect();
		stats.quads = quads.len();
		self.sprites.draw(gpu, target, uniforms, Some(clear), &quads, &ready);
		stats
	}
}

/// A loaded font as the world quads take it.
fn label_font(font: &Option<(BitmapFont, String)>) -> Option<LabelFont<'_>> {
	font.as_ref()
		.map(|(font, texture)| LabelFont { font, texture })
}

/// An ARGB colour as RGBA in `0..=1`.
fn argb_floats(argb: u32) -> [f32; 4] {
	let [a, r, g, b] = argb.to_be_bytes();
	[r, g, b, a].map(|c| f32::from(c) / 255.0)
}
