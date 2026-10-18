//! [`WorldRenderer`]: draws the world map frame by frame, loading atlases on demand.
//!
//! No preload list is read. An atlas starts decoding on a
//! worker thread when a script hints at it (a brush set on a sprite, tile or detail,
//! or `Tactical.addResource`; see `squirrel_bb::Sprites::take_hints`) or when a frame
//! first needs it, and goes to the GPU when a frame draws it; what no frame draws for a
//! while is dropped again ([`crate::residency`]). A live frame skips sprites whose atlas
//! isn't ready yet, and they appear once it is; a frame for a screenshot waits for every
//! atlas instead.

use std::sync::Arc;

use bb_assets::{BitmapFont, FileSource, Image, ImageLoader, Slot, SpriteRegistry, TextureCache};
use squirrel_bb::FogPatch;
use squirrel_bb::scene::WorldScene;
use squirrel_bb::scene::tactical::TacticalScene;

use crate::fog::FogPass;
use crate::gpu::Gpu;
use crate::fonts::shaper::{GLYPH_PAGE, TextShaper, glyph_page};
use crate::fonts::{FontDb, Registration};
use crate::labels::{LabelFont, WHITE};
use crate::mips::{MipMaker, samples_mips};
use crate::pages::{FramePages, Located};
use crate::residency::{Drawer, Residency};
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

/// A scene the game discarded, whose textures go (see [`crate::residency`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discard {
	/// A battle ended (`Tactical.clearScene`).
	Battle,
	/// The campaign was left (`World.clearScene`).
	Campaign,
}

/// What one frame drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
	pub quads: usize,
	/// Quads skipped because their atlas was still loading.
	pub waiting: usize,
	/// Atlases that failed to load: drawn with the "texture missing" picture, or
	/// skipped without it.
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
	/// Whether a battle draws with the depth of field (see [`WorldRenderer::set_depth_of_field`]).
	depth_of_field: bool,
	/// Which parts of the atlases are drawn, when tracked ([`WorldRenderer::track_usage`]).
	usage: Option<crate::usage::TextureUsage>,
	/// When each texture was last drawn, for which stay on the GPU.
	residency: Residency,
	/// Makes an atlas's mips when a draw first samples them.
	mips: MipMaker,
	/// The drawn frames in our own pages (`None`: whole atlases, `BB_TEXTURE_PAGES=off`).
	pages: Option<FramePages>,
	/// Text the bitmap fonts can't draw, shaped in the outline fonts.
	text: std::cell::RefCell<TextShaper>,
	/// The glyph pages' textures (see `fonts::shaper`).
	glyph_pages: Vec<GpuTexture>,
}

/// The game's own outline fonts, the bitmap fonts' sources.
const GAME_FONT_FILES: [&str; 3] = [
	"gfx/fonts/cinzel/Cinzel-Regular.ttf",
	"gfx/fonts/cinzel/Cinzel-Bold.ttf",
	"gfx/fonts/cinzel/Cinzel-Black.ttf",
];

/// Where a quad is drawn from this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum From {
	/// The renderer's white texel (label plates).
	White,
	/// Its atlas, whole: the map's atlas at this index.
	Whole(usize),
	/// One of our pages.
	Page(usize),
	/// One of the glyph pages.
	Glyph(usize),
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

/// What a texture that fails to load shows instead: the game's "texture missing"
/// picture (a 128×128 rainbow), as the original's (see `bb_assets::TextureCache`).
const ERROR_TEXTURE: &str = "error.png";

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
			textures: TextureCache::new(ImageLoader::new(files.clone(), threads)).with_fallback(ERROR_TEXTURE),
			label_font,
			region_font,
			white: GpuTexture::upload(gpu, "white", &white),
			fog: FogPass::new(gpu),
			water: WaterPass::new(gpu),
			clock: Clock::default(),
			depth_of_field: false,
			usage: None,
			residency: Residency::new(gpu.texture_budget()),
			mips: MipMaker::new(gpu),
			pages: (!std::env::var_os("BB_TEXTURE_PAGES").is_some_and(|v| v == "off" || v == "0"))
				.then(|| FramePages::new(files.clone(), bb_assets::SheetCache::from_env(), threads)),
			text: std::cell::RefCell::new(TextShaper::new(FontDb::new(
				GAME_FONT_FILES.iter().filter_map(|path| files.read(path)).collect(),
				true,
			))),
			glyph_pages: Vec::new(),
		}
	}

	/// A mod's font (`ModernEngine.registerFont`): its family, or why it can't be used.
	pub fn register_font(&mut self, font: Registration) -> Result<String, String> {
		self.text.get_mut().db().register(font)
	}

	/// Put the glyphs shaped since the last frame in their pages, making pages as needed.
	fn upload_glyphs(&mut self, gpu: &Gpu) {
		let shaper = self.text.get_mut();
		let uploads = shaper.take_uploads();
		while self.glyph_pages.len() < shaper.pages() {
			let empty = Image {
				width: GLYPH_PAGE,
				height: GLYPH_PAGE,
				rgba: vec![0; (GLYPH_PAGE * GLYPH_PAGE * 4) as usize],
			};
			self.glyph_pages.push(GpuTexture::upload_flat(gpu, "glyphs", &empty));
		}
		for glyph in uploads {
			let Some(page) = self.glyph_pages.get(glyph.page) else { continue };
			gpu.queue.write_texture(
				wgpu::TexelCopyTextureInfo {
					texture: &page.texture,
					mip_level: 0,
					origin: wgpu::Origin3d { x: glyph.x, y: glyph.y, z: 0 },
					aspect: wgpu::TextureAspect::All,
				},
				&glyph.rgba,
				wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(glyph.width * 4),
					rows_per_image: Some(glyph.height),
				},
				wgpu::Extent3d { width: glyph.width, height: glyph.height, depth_or_array_layers: 1 },
			);
		}
	}

	/// Take a change to the fog of war's mask (from `WorldGrid::take_fog_patch`).
	pub fn update_fog(&mut self, gpu: &Gpu, patch: &FogPatch) {
		self.fog.update_mask(gpu, patch);
	}

	/// What it holds on the GPU: every texture loaded by path, the fog mask and its own
	/// (for the memory inventory).
	pub fn memory(&self) -> crate::RendererMemory {
		let mut memory = crate::RendererMemory::of_cache(&self.textures);
		if let Some(pages) = &self.pages {
			(memory.pages, memory.frames, memory.page_bytes) = pages.memory();
		}
		memory.fog_mask = self.fog.bytes();
		memory.own = self.white.bytes() + self.sprites.bytes();
		memory
	}

	/// Track which parts of the atlases the draws sample from now on (for the texture
	/// residency design; a cost per quad, so off by default).
	pub fn track_usage(&mut self) {
		self.usage.get_or_insert_with(Default::default);
	}

	/// The atlases' use since tracking started, the most drawn first.
	pub fn usage(&self) -> Option<Vec<crate::AtlasUse>> {
		self.usage.as_ref().map(crate::usage::TextureUsage::report)
	}

	/// The atlases' use since tracking started (or since the last take), and start over:
	/// what one stretch of play draws.
	pub fn take_usage(&mut self) -> Option<Vec<crate::AtlasUse>> {
		self.usage.as_mut().map(|usage| std::mem::take(usage).report())
	}

	fn record_usage(&mut self, map: &crate::world::WorldQuads, zoom: f32, dof_below: Option<u32>) {
		let Some(usage) = self.usage.as_mut() else { return };
		let textures = &self.textures;
		usage.record(map, zoom, dof_below, |path| {
			textures.slot(path).and_then(Slot::texture).map(|t| (t.width, t.height))
		});
	}

	/// The game's clocks: the fog drifts with real time, the water with virtual time.
	pub fn set_clock(&mut self, clock: Clock) {
		self.clock = clock;
	}

	/// Draw battles with the depth of field (off until set, as the original's default):
	/// the video mode's `DepthOfField`. The original draws a battle's tiles with its `SpriteDOF`
	/// program while it's on, and with the plain `Sprite` program while it's off (the
	/// tile layer's flag 4, which `setVideoMode` sets, `0x500905`, and the draw reads,
	/// `0x557cc0`). Its default is off (`0x499d93`).
	pub fn set_depth_of_field(&mut self, on: bool) {
		self.depth_of_field = on;
	}

	/// Get `paths` (the scripts' hints) ready in the background for a draw: an atlas
	/// the pages draw from has its sheet made ready (see [`crate::pages`]), any other
	/// image is decoded for a draw to upload (see [`crate::residency`]). Only images
	/// are loaded here; other hinted resources, such as sounds, are left to their own
	/// subsystem.
	pub fn prefetch(&mut self, registry: &SpriteRegistry, paths: impl IntoIterator<Item = String>) {
		self.index(registry);
		for path in paths.into_iter().filter(|p| p.ends_with(".png")) {
			let paged = !path.starts_with(FONT_DIR) && self.pages.as_mut().is_some_and(|p| p.warm(&path));
			if !paged {
				self.textures.warm(&path);
			}
		}
	}

	/// A new frame: what it draws counts as drawn now (see [`crate::residency`]).
	pub fn begin_frame(&mut self) {
		self.residency.next_frame();
		if let Some(pages) = &mut self.pages {
			pages.begin_frame(self.residency.frame());
		}
	}

	/// Draw from whole atlases from now on, as before our pages (`crate::pages`; for
	/// comparing the two, and the tests of the atlases' own path).
	pub fn draw_whole_atlases(&mut self) {
		self.pages = None;
	}

	/// Index the atlases' frames for the pages (once, and again if the brushes changed).
	fn index(&mut self, registry: &SpriteRegistry) {
		if let Some(pages) = &mut self.pages {
			pages.index(registry);
		}
	}

	/// Where each of `map`'s quads is drawn from this frame, in order (`None`: it waits
	/// for its frame or atlas), drawn at `view` (the zoom, the depth of field's level) for
	/// `drawer`. Frames and atlases not on the GPU are asked for, or loaded now for a draw
	/// that waits for all.
	fn resolve(
		&mut self,
		gpu: &Gpu,
		map: &crate::world::WorldQuads,
		atlases: Atlases,
		drawer: Drawer,
		view: (f32, Option<u32>),
	) -> Vec<Option<(From, Quad)>> {
		let upload = |path: &str, image: bb_assets::Image| upload(gpu, path, &image);
		self.textures.pump(upload);
		let wait = atlases == Atlases::WaitForAll;
		let mut whole = vec![false; map.atlases.paths.len()];
		let mut out = Vec::with_capacity(map.quads.len());
		{
			let (pages, mips) = (&mut self.pages, &self.mips);
			if let Some(pages) = pages.as_mut() {
				pages.pump(gpu, mips);
			}
			let locate = |pages: &mut Option<FramePages>, quad: &Quad, whole: &mut [bool]| -> Option<(From, Quad)> {
				let path = &map.atlases.paths[quad.texture];
				if path == WHITE {
					return Some((From::White, *quad));
				}
				if let Some(page) = glyph_page(path) {
					return Some((From::Glyph(page), *quad));
				}
				if let Some(pages) = pages.as_mut().filter(|_| !path.starts_with(FONT_DIR)) {
					match pages.locate(path, quad, drawer, wait) {
						Located::Page { page, uv } => return Some((From::Page(page), Quad { uv, ..*quad })),
						Located::Waiting => return None,
						Located::Whole => {}
					}
				}
				whole[quad.texture] = true;
				Some((From::Whole(quad.texture), *quad))
			};
			for quad in &map.quads {
				out.push(locate(pages, quad, &mut whole));
			}
			// What this frame lacks is loaded now (a draw that waits for all), or given
			// the pages' wait budget.
			let asked = match pages.as_mut() {
				Some(p) if wait => {
					p.load_asks_now(gpu, mips);
					true
				}
				Some(p) => {
					let asked = p.send_asks();
					if !asked.is_empty() {
						p.pump_until(gpu, mips, &asked, std::time::Instant::now() + crate::pages::WAIT_BUDGET);
					}
					!asked.is_empty()
				}
				None => false,
			};
			if asked {
				for (slot, quad) in out.iter_mut().zip(&map.quads) {
					if slot.is_none() {
						*slot = locate(pages, quad, &mut whole);
					}
				}
			}
		}
		for (path, _) in map.atlases.paths.iter().zip(&whole).filter(|(p, w)| **w && *p != WHITE) {
			match atlases {
				Atlases::WaitForAll => {
					let _ = self.textures.get_now(path, upload);
				}
				Atlases::DrawReady => {
					let _ = self.textures.get_or_upload(path, upload);
				}
			}
			self.residency.drawn(path, drawer);
		}
		self.add_mips(gpu, map, view.0, view.1);
		out
	}

	/// The textures `resolved` draws from, and its quads pointing at them in order
	/// (`None`: not drawn: waiting, or its atlas failed with no stand-in).
	fn bind<'a>(
		&'a self,
		map: &crate::world::WorldQuads,
		resolved: &[Option<(From, Quad)>],
	) -> (Vec<&'a GpuTexture>, Vec<Option<Quad>>, FrameStats) {
		let mut stats = FrameStats::default();
		let mut ready: Vec<&'a GpuTexture> = Vec::new();
		let mut slots: std::collections::HashMap<From, Option<usize>> = std::collections::HashMap::new();
		let mut counted = std::collections::HashSet::new();
		let quads = resolved
			.iter()
			.map(|entry| {
				let Some((from, quad)) = entry else {
					stats.waiting += 1;
					return None;
				};
				let slot = *slots.entry(*from).or_insert_with(|| {
					let texture = match from {
						From::White => Some(&self.white),
						From::Glyph(page) => self.glyph_pages.get(*page),
						From::Page(page) => self.pages.as_ref().map(|p| p.texture(*page)),
						From::Whole(i) => {
							let path = &map.atlases.paths[*i];
							if matches!(self.textures.slot(path), Some(Slot::Missing(..) | Slot::Failed(_))) && counted.insert(*i) {
								stats.failed += 1;
							}
							self.textures.slot(path).and_then(Slot::texture)
						}
					}?;
					ready.push(texture);
					Some(ready.len() - 1)
				});
				match slot {
					Some(texture) => Some(Quad { texture, ..*quad }),
					None => {
						stats.waiting += 1;
						None
					}
				}
			})
			.collect();
		(ready, quads, stats)
	}


	/// The bytes of textures kept before the least recently drawn go.
	pub fn set_texture_budget(&mut self, bytes: u64) {
		self.residency.set_budget(bytes);
	}

	/// The game discarded a scene: what only it needed goes, and the warmed images no
	/// draw asked for.
	pub fn discard(&mut self, discard: Discard) {
		self.textures.drop_all_decoded();
		if let Some(pages) = &mut self.pages {
			match discard {
				Discard::Battle => pages.battle_discarded(),
				Discard::Campaign => pages.campaign_discarded(),
			}
		}
		let resident = self.resident();
		let gone = match discard {
			Discard::Battle => self.residency.battle_discarded(&resident),
			Discard::Campaign => self.residency.campaign_discarded(&resident),
		};
		for path in gone {
			self.textures.remove(&path);
		}
	}

	/// The textures on the GPU by path, with their bytes. The "texture missing"
	/// stand-ins aren't counted: a dropped one would only fail again.
	fn resident(&self) -> Vec<(String, u64)> {
		self.textures
			.slots()
			.filter_map(|(path, slot)| match slot {
				Slot::Ready(texture) => Some((path.to_string(), texture.bytes())),
				_ => None,
			})
			.collect()
	}

	/// Give the atlases `map`'s quads sample the mips of (drawn at `zoom`, the depth of
	/// field blurring below `dof_below`) their mips, before they're drawn.
	fn add_mips(&mut self, gpu: &Gpu, map: &crate::world::WorldQuads, zoom: f32, dof_below: Option<u32>) {
		let mut need = vec![false; map.atlases.paths.len()];
		for quad in &map.quads {
			if need[quad.texture] {
				continue;
			}
			let path = &map.atlases.paths[quad.texture];
			// A font never has mips: drawn smaller, it samples its full size, as the
			// original's do (see `crate::texture`).
			if path.starts_with(FONT_DIR) {
				continue;
			}
			let Some(texture) = self.textures.slot(path).and_then(Slot::texture) else { continue };
			if texture.texture.mip_level_count() == 1 && samples_mips(quad, zoom, (texture.width, texture.height), dof_below) {
				need[quad.texture] = true;
			}
		}
		let (mips, textures) = (&self.mips, &mut self.textures);
		for (path, _) in map.atlases.paths.iter().zip(need).filter(|(_, need)| *need) {
			textures.update(path, |texture| Some(mips.with_mips(gpu, texture)));
		}
	}

	/// Keep the textures within the budget, and the warmed images too, the oldest
	/// going first.
	fn fit_budget(&mut self) {
		let resident = self.resident();
		for path in self.residency.over_budget(&resident) {
			self.textures.remove(&path);
		}
		// The pages share what the whole textures leave of the budget.
		let whole: u64 = self.resident().iter().map(|(_, bytes)| bytes).sum();
		if let Some(pages) = &mut self.pages {
			pages.fit(self.residency.budget().saturating_sub(whole));
		}
		let mut held = self.textures.decoded();
		let mut total: u64 = held.iter().map(|(_, _, bytes)| bytes).sum();
		held.sort_by_key(|(_, since, _)| *since);
		for (path, _, bytes) in held {
			if total <= self.residency.budget() {
				break;
			}
			total -= bytes;
			self.textures.remove(&path);
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
		let upload = |path: &str, image: bb_assets::Image| upload(gpu, path, &image);
		self.textures.pump(upload);
		let fonts = WorldFonts {
			label: label_font(&self.label_font),
			region: label_font(&self.region_font),
			shaper: Some(&self.text),
		};
		let world = world_quads(scene, registry, view, fonts);
		self.upload_glyphs(gpu);
		self.record_usage(&world, view.zoom, None);
		self.index(registry);
		for path in [WATER_TEXTURE, FOG_TEXTURE].into_iter().filter(|p| *p == WATER_TEXTURE || scene.fog_of_war) {
			match atlases {
				Atlases::WaitForAll => {
					let _ = self.textures.get_now(path, upload);
				}
				Atlases::DrawReady => {
					let _ = self.textures.get_or_upload(path, upload);
				}
			}
			self.residency.drawn(path, Drawer::World);
		}
		let resolved = self.resolve(gpu, &world, atlases, Drawer::World, (view.zoom, None));
		let (ready, quads, mut stats) = self.bind(&world, &resolved);
		let map: Vec<Quad> = quads[..world.labels_from].iter().flatten().copied().collect();
		let labels: Vec<Quad> = quads[world.labels_from..].iter().flatten().copied().collect();
		stats.quads = map.len() + labels.len();
		// The day's light: on lit sprites, the water and the fog.
		let ambient = argb_floats(scene.light.color);
		let uniforms = SceneUniforms {
			ambient_color: ambient,
			ambient_saturation: scene.light.saturation,
			..view.uniforms()
		};
		let water = match self.textures.slot(WATER_TEXTURE).and_then(Slot::texture) {
			Some(water) => self.water.draw(
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
			if let Some(fog_texture) = self.textures.slot(FOG_TEXTURE).and_then(Slot::texture) {
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
		self.fit_budget();
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
		self.index(registry);
		let camera_level = scene.camera.level.clamp(0, 3) as u32;
		self.record_usage(&map, view.zoom, self.depth_of_field.then_some(camera_level));
		// The battle's light (the weather's `setAmbientLighting*`), and the depth of
		// field below the camera's level.
		let uniforms = SceneUniforms {
			ambient_color: argb_floats(scene.light.color),
			ambient_saturation: scene.light.saturation,
			camera_level: scene.camera.level.clamp(0, 3) as f32,
			dof_strength: crate::tactical::dof_strength(self.depth_of_field),
			..view.uniforms()
		};
		let dof_below = self.depth_of_field.then_some(camera_level);
		let stats = self.draw_quads(gpu, target, &map, &uniforms, Some(CLEAR), atlases, Drawer::Battle, (view.zoom, dof_below));
		self.fit_budget();
		stats
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
		self.record_usage(&map, 1.0, None);
		self.index(registry);
		let target = RenderTarget::new(gpu, width, height);
		let uniforms = SceneUniforms::centered([0.0, 0.0], width as f32, height as f32);
		self.draw_quads(gpu, &target, &map, &uniforms, Some([0.0; 4]), Atlases::WaitForAll, Drawer::Other, (1.0, None));
		target.read(gpu).map(Some)
	}

	/// Draw the mouse cursor's sprite `brush` over what `target` holds, its frame `since`
	/// seconds into its animation, at `at`: screen pixels from the centre, y up, as the
	/// original draws its software cursor (last, over the UI, unscaled; see
	/// `squirrel_bb::native::ui_cursor`). An unknown brush, or an atlas still loading,
	/// draws nothing.
	pub fn draw_cursor(
		&mut self,
		gpu: &Gpu,
		target: &RenderTarget,
		registry: &SpriteRegistry,
		brush: &str,
		since: f32,
		at: [f32; 2],
	) -> FrameStats {
		let Some(map) = crate::cursor::cursor_quads(registry, brush, since, at) else {
			return FrameStats::default();
		};
		let uniforms = SceneUniforms::centered([0.0, 0.0], target.width as f32, target.height as f32);
		self.record_usage(&map, 1.0, None);
		self.index(registry);
		self.draw_quads(gpu, target, &map, &uniforms, None, Atlases::DrawReady, Drawer::Other, (1.0, None))
	}

	/// Draw `map`'s quads into `target`, over `clear` or what it holds, with the atlases
	/// they use as ready (or waited for).
	fn draw_quads(
		&mut self,
		gpu: &Gpu,
		target: &RenderTarget,
		map: &crate::world::WorldQuads,
		uniforms: &SceneUniforms,
		clear: Option<[f64; 4]>,
		atlases: Atlases,
		drawer: Drawer,
		(zoom, dof_below): (f32, Option<u32>),
	) -> FrameStats {
		let resolved = self.resolve(gpu, map, atlases, drawer, (zoom, dof_below));
		let (ready, quads, mut stats) = self.bind(map, &resolved);
		let quads: Vec<Quad> = quads.into_iter().flatten().collect();
		stats.quads = quads.len();
		self.sprites.draw(gpu, target, uniforms, clear, &quads, &ready);
		stats
	}
}

/// A texture as it goes to the GPU: an atlas or a font with level 0 only (an atlas's
/// mips are made when a draw first samples them, see [`crate::mips`]), the water's and
/// the fog's with their mips (their passes tile them over the view).
fn upload(gpu: &Gpu, path: &str, image: &Image) -> GpuTexture {
	if path == WATER_TEXTURE || path == FOG_TEXTURE {
		GpuTexture::upload(gpu, path, image)
	} else {
		GpuTexture::upload_flat(gpu, path, image)
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
