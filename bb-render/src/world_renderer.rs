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
use squirrel_bb::scene::{SceneDetail, SceneTile, WorldScene};
use squirrel_bb::scene::tactical::TacticalScene;

use crate::fast_hash::FastMap;
use crate::fog::FogPass;
use crate::kept_map::{self, Built, KeptMap, Placing};
use crate::kept_sprites::{self, KeptSprites, Locating};
use crate::gpu::Gpu;
use crate::fonts::shaper::{GLYPH_PAGE, TextShaper, glyph_page};
use crate::fonts::{FontDb, Registration};
use crate::labels::{LabelFont, WHITE};
use crate::mips::{MipMaker, samples_mips};
use crate::pages::{FramePages, Located};
use crate::residency::{Drawer, Residency};
use crate::sprite::{Part, Prepared, Quad, SceneUniforms, SpriteRenderer, begin_pass};
use crate::tactical::tactical_quads;
use crate::target::RenderTarget;
use crate::texture::GpuTexture;
use crate::water::WaterPass;
use crate::world::{QuadScratch, WorldFonts, WorldView, world_quads_around_map};

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
	/// A frame's quads as they're built and located, kept from frame to frame: zoomed
	/// out, the map is tens of thousands of sprites, megabytes a frame to allocate.
	quad_scratch: QuadScratch,
	frame: FrameQuads,
	/// The world map's terrain and details, kept from frame to frame.
	kept: KeptMap,
	/// The world map's moving layer, kept from frame to frame (see [`crate::kept_sprites`]).
	moving: KeptSprites,
	/// The fonts mods registered: text shaped before one came is shaped again.
	fonts_registered: usize,
}

/// How a world frame draws its moving layer (entities, labels, effects).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Moving {
	/// Built for this frame, streamed.
	Streamed,
	/// Kept on the GPU (see [`crate::kept_sprites`]).
	Kept,
}

/// A draw's quads located (see [`WorldRenderer::resolve`]) and bound (see
/// [`WorldRenderer::bind`]), in parts drawn in turn.
#[derive(Default)]
struct FrameQuads {
	resolved: Vec<Option<(From, Quad)>>,
	parts: [Vec<Quad>; 3],
}

/// The world map for [`WorldRenderer::draw_kept`]: what it's kept from.
pub struct WorldMap<'a> {
	/// The map's revision (`WorldGrid::map_revision`): its terrain and details are built
	/// again when it changes.
	pub revision: u64,
	/// The whole map (`WorldGrid::scene_map`), asked for only then.
	pub tiles: &'a dyn Fn() -> Vec<SceneTile>,
	/// The details whose look changed since the last frame
	/// (`WorldGrid::take_detail_changes`).
	pub changes: &'a [SceneDetail],
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

/// What an atlas of a frame's list is drawn as.
#[derive(Clone, Copy)]
enum Kind {
	White,
	Glyph(usize),
	/// From our pages (unless they give it up as whole).
	Paged,
	Whole,
}

/// A quad's texture coordinates as bits, for keying.
fn uv_bits(uv: &[[f32; 2]; 4]) -> [u32; 8] {
	let mut out = [0; 8];
	for (i, [u, v]) in uv.iter().enumerate() {
		out[2 * i] = u.to_bits();
		out[2 * i + 1] = v.to_bits();
	}
	out
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
pub(crate) const FONT_DIR: &str = "gfx/fonts/";

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
			quad_scratch: QuadScratch::default(),
			frame: FrameQuads::default(),
			kept: KeptMap::default(),
			moving: KeptSprites::default(),
			fonts_registered: 0,
		}
	}

	/// A mod's font (`ModernEngine.registerFont`): its family, or why it can't be used.
	pub fn register_font(&mut self, font: Registration) -> Result<String, String> {
		self.fonts_registered += 1;
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
		out: &mut Vec<Option<(From, Quad)>>,
	) -> bool {
		let upload = |path: &str, image: bb_assets::Image| upload(gpu, path, &image);
		self.textures.pump(upload);
		let wait = atlases == Atlases::WaitForAll;
		let mut whole = vec![false; map.atlases.paths.len()];
		let asked;
		out.clear();
		out.reserve(map.quads.len());
		{
			let (pages, mips) = (&mut self.pages, &self.mips);
			if let Some(pages) = pages.as_mut() {
				pages.pump(gpu, mips);
			}
			// What each atlas is drawn as, worked out once a frame, not once a quad.
			let kinds: Vec<Kind> = map
				.atlases
				.paths
				.iter()
				.map(|path| {
					if path == WHITE {
						Kind::White
					} else if let Some(page) = glyph_page(path) {
						Kind::Glyph(page)
					} else if pages.is_some() && !path.starts_with(FONT_DIR) {
						Kind::Paged
					} else {
						Kind::Whole
					}
				})
				.collect();
			// Where each distinct frame is, asked of the pages once a frame: quads of one
			// frame are located alike (asking again changes nothing).
			let mut located: FastMap<(usize, [u32; 8]), Located> = FastMap::default();
			let locate = |pages: &mut Option<FramePages>,
			              located: &mut FastMap<(usize, [u32; 8]), Located>,
			              quad: &Quad,
			              whole: &mut [bool]|
			 -> Option<(From, Quad)> {
				match kinds[quad.texture] {
					Kind::White => return Some((From::White, *quad)),
					Kind::Glyph(page) => return Some((From::Glyph(page), *quad)),
					Kind::Paged => {
						let pages = pages.as_mut().expect("paged");
						let path = &map.atlases.paths[quad.texture];
						let uv = quad.uv;
						let key = (quad.texture, uv_bits(&uv));
						let at = *located.entry(key).or_insert_with(|| pages.locate(path, quad, drawer, wait));
						match at {
							Located::Page { page, map } => return Some((From::Page(page), Quad { uv: map.uv(uv), ..*quad })),
							Located::Waiting => return None,
							Located::Whole => {}
						}
					}
					Kind::Whole => {}
				}
				whole[quad.texture] = true;
				Some((From::Whole(quad.texture), *quad))
			};
			for quad in &map.quads {
				out.push(locate(pages, &mut located, quad, &mut whole));
			}
			// What this frame lacks is loaded now (a draw that waits for all), or given
			// the pages' wait budget.
			asked = match pages.as_mut() {
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
				// Frames that came in since are asked about again.
				located.retain(|_, at| !matches!(at, Located::Waiting));
				for (slot, quad) in out.iter_mut().zip(&map.quads) {
					if slot.is_none() {
						*slot = locate(pages, &mut located, quad, &mut whole);
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
		asked
	}

	/// The textures `resolved` draws from, and its quads pointing at them in order, into
	/// `parts`: those before `splits[0]` into the first, those from there to `splits[1]`
	/// into the second, and so on (a quad not drawn, waiting or its atlas failed with no
	/// stand-in, is left out).
	fn bind<'a>(
		&'a self,
		map: &crate::world::WorldQuads,
		resolved: &[Option<(From, Quad)>],
		splits: &[usize],
		parts: &mut [Vec<Quad>],
	) -> (Vec<&'a GpuTexture>, FrameStats) {
		let mut stats = FrameStats::default();
		let mut ready: Vec<&'a GpuTexture> = Vec::new();
		let mut slots: FastMap<From, Option<usize>> = FastMap::default();
		let mut counted = std::collections::HashSet::new();
		for part in parts.iter_mut() {
			part.clear();
		}
		for (i, entry) in resolved.iter().enumerate() {
			let Some((from, quad)) = entry else {
				stats.waiting += 1;
				continue;
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
				Some(texture) => parts[splits.iter().filter(|&&s| s <= i).count()].push(Quad { texture, ..*quad }),
				None => stats.waiting += 1,
			}
		}
		(ready, stats)
	}


	/// The bytes of textures kept before the least recently drawn go.
	pub fn set_texture_budget(&mut self, bytes: u64) {
		self.residency.set_budget(bytes);
	}

	/// The game discarded a scene: what only it needed goes, and the warmed images no
	/// draw asked for.
	pub fn discard(&mut self, discard: Discard) {
		// The kept map's and moving layer's bind groups hold their textures.
		self.kept.forget_groups();
		self.moving.forget_groups();
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
		// Each atlas's whole texture, if it has one without mips yet, looked up once. A font
		// never has mips: drawn smaller, it samples its full size, as the original's do
		// (see `crate::texture`).
		let sizes: Vec<Option<(u32, u32)>> = map
			.atlases
			.paths
			.iter()
			.map(|path| {
				if path.starts_with(FONT_DIR) {
					return None;
				}
				let texture = self.textures.slot(path).and_then(Slot::texture)?;
				(texture.texture.mip_level_count() == 1).then_some((texture.width, texture.height))
			})
			.collect();
		for quad in &map.quads {
			if need[quad.texture] {
				continue;
			}
			if let Some(size) = sizes[quad.texture]
				&& samples_mips(quad, zoom, size, dof_below)
			{
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
		let over = self.residency.over_budget(&resident);
		if !over.is_empty() {
			// The kept map's and moving layer's bind groups hold their textures.
			self.kept.forget_groups();
			self.moving.forget_groups();
		}
		for path in over {
			self.textures.remove(&path);
		}
		// The pages share what the whole textures leave of the budget.
		let whole: u64 = self.resident().iter().map(|(_, bytes)| bytes).sum();
		if let Some(pages) = &mut self.pages {
			pages.fit(self.residency.budget().saturating_sub(whole));
			self.kept.follow_generation(pages.generation());
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
	/// then the labels. The map is `scene`'s tiles, built for this frame only (see
	/// [`draw_kept`](Self::draw_kept) for a map kept from frame to frame).
	pub fn draw(
		&mut self,
		gpu: &Gpu,
		target: &RenderTarget,
		scene: &WorldScene,
		registry: &SpriteRegistry,
		view: &WorldView,
		atlases: Atlases,
	) -> FrameStats {
		self.kept.build(&scene.tiles, registry, None);
		self.draw_world(gpu, target, scene, registry, view, atlases, Moving::Streamed)
	}

	/// [`draw`](Self::draw) with the map's terrain and details kept on the GPU from frame
	/// to frame (see [`crate::kept_map`]): built again only when `map`'s revision (or the
	/// registry) changes, the details changed written again. The moving layer is kept
	/// too, its items built again only when they change (see [`crate::kept_sprites`]).
	/// `scene`'s own tiles aren't drawn (a scene without them,
	/// `Engine::world_scene_where(|_| false)`, copies nothing).
	pub fn draw_kept(
		&mut self,
		gpu: &Gpu,
		target: &RenderTarget,
		scene: &WorldScene,
		map: &WorldMap,
		registry: &SpriteRegistry,
		view: &WorldView,
		atlases: Atlases,
	) -> FrameStats {
		let built = Some(Built {
			revision: map.revision,
			sheets: registry.sheets().len(),
		});
		if self.kept.stale(built) {
			let _span = crate::frame_timing::span("map built");
			let tiles = (map.tiles)();
			self.kept.build(&tiles, registry, built);
		}
		self.kept.change(map.changes, registry);
		self.draw_world(gpu, target, scene, registry, view, atlases, Moving::Kept)
	}

	#[allow(clippy::too_many_arguments)]
	fn draw_world(
		&mut self,
		gpu: &Gpu,
		target: &RenderTarget,
		scene: &WorldScene,
		registry: &SpriteRegistry,
		view: &WorldView,
		atlases: Atlases,
		moving: Moving,
	) -> FrameStats {
		let upload = |path: &str, image: bb_assets::Image| upload(gpu, path, &image);
		self.textures.pump(upload);
		let fonts = WorldFonts {
			label: label_font(&self.label_font),
			region: label_font(&self.region_font),
			shaper: Some(&self.text),
		};
		let world = {
			let _span = crate::frame_timing::span("world quads");
			match moving {
				Moving::Streamed => Some(world_quads_around_map(scene, registry, view, fonts, &mut self.quad_scratch)),
				Moving::Kept => {
					let built = kept_sprites::Built {
						sheets: registry.sheets().len(),
						fonts: self.fonts_registered,
					};
					self.moving.update(scene, view, registry, fonts, built, self.pages.is_some());
					None
				}
			}
		};
		self.upload_glyphs(gpu);
		if let Some(world) = &world {
			self.record_usage(world, view.zoom, None);
		}
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
		let resolve_span = crate::frame_timing::span("resolve+bind");
		// The kept map's segments on screen, located (those not yet, or anew) before the
		// rest, so that the frames either asks for are waited for together.
		if self.sprites.keep(gpu, self.kept.len()) {
			self.kept.unplace();
		}
		if moving == Moving::Kept && self.sprites.keep_instances(gpu, self.moving.places()) {
			self.moving.unwritten();
		}
		if let Some(pages) = &mut self.pages {
			pages.pump(gpu, &self.mips);
		}
		let generation = |pages: &Option<FramePages>| pages.as_ref().map_or(0, FramePages::generation);
		let at = generation(&self.pages);
		self.kept.begin_frame(at);
		self.moving.begin_frame(at);
		let mut shown = self.kept.shown(view);
		let wait = atlases == Atlases::WaitForAll;
		let whole_path = |path: &str| path.starts_with(FONT_DIR);
		self.kept.place(&mut shown, at, &mut Placing {
			pages: self.pages.as_mut(),
			wait,
			sprites: &self.sprites,
			whole: &whole_path,
		});
		let mut frame = std::mem::take(&mut self.frame);
		let asked = match &world {
			Some(world) => self.resolve(gpu, world, atlases, Drawer::World, (view.zoom, None), &mut frame.resolved),
			None => {
				let mut locating = Locating {
					pages: self.pages.as_mut(),
					wait,
					generation: at,
				};
				let waiting = self.moving.locate(&mut locating, &self.sprites);
				self.ask(gpu, wait) && (waiting > 0 || shown.waiting > 0)
			}
		};
		let now = generation(&self.pages);
		if (asked && shown.waiting > 0) || now != at || (asked && world.is_none()) {
			self.kept.begin_frame(now);
			self.kept.frames_came();
			self.kept.place(&mut shown, now, &mut Placing {
				pages: self.pages.as_mut(),
				wait,
				sprites: &self.sprites,
				whole: &whole_path,
			});
			if world.is_none() {
				self.moving.begin_frame(now);
				self.moving.frames_came();
				self.moving.locate(
					&mut Locating {
						pages: self.pages.as_mut(),
						wait,
						generation: now,
					},
					&self.sprites,
				);
			}
		}
		let (kept_failed, kept_unready) = self.kept_atlases(gpu, &shown, atlases, view.zoom);
		let moving_failed = if world.is_none() { self.moving_atlases(gpu, atlases, view.zoom) } else { 0 };
		{
			let (pages, textures, blank) = (&self.pages, &self.textures, self.sprites.blank_view());
			let paths = self.kept.atlas_paths().to_vec();
			self.kept.bind(&shown, &self.sprites, gpu, |from| match from {
				kept_map::From::Page(page) => &pages.as_ref().expect("paged").texture(page).view,
				kept_map::From::Whole(atlas) => textures.slot(&paths[atlas]).and_then(Slot::texture).map_or(blank, |t| &t.view),
			});
		}
		let (ready, mut stats) = match &world {
			Some(world) => self.bind(world, &frame.resolved, &[world.details_end, world.labels_from], &mut frame.parts),
			None => {
				let (pages, textures, white, glyphs) = (&self.pages, &self.textures, &self.white, &self.glyph_pages);
				let paths = self.moving.atlas_paths().to_vec();
				let drawn = self.moving.bind(&self.sprites, gpu, |from| match from {
					kept_sprites::From::White => Some(&white.view),
					kept_sprites::From::Glyph(page) => glyphs.get(page).map(|t| &t.view),
					kept_sprites::From::Page(page) => Some(&pages.as_ref().expect("paged").texture(page).view),
					kept_sprites::From::Whole(atlas) => textures.slot(&paths[atlas]).and_then(Slot::texture).map(|t| &t.view),
				});
				let stats = FrameStats {
					quads: drawn.quads,
					waiting: drawn.waiting,
					failed: moving_failed,
				};
				(Vec::new(), stats)
			}
		};
		drop(resolve_span);
		stats.quads += shown.quads - kept_unready;
		stats.waiting += shown.waiting + kept_unready;
		stats.failed += kept_failed;
		if world.is_some() {
			stats.quads = frame.parts.iter().map(Vec::len).sum::<usize>() + shown.quads - kept_unready;
		}
		// The day's light: on lit sprites, the water and the fog.
		let ambient = argb_floats(scene.light.color);
		let uniforms = SceneUniforms {
			ambient_color: ambient,
			ambient_saturation: scene.light.saturation,
			..view.uniforms()
		};
		let sprites_span = crate::frame_timing::span("sprites draw");
		// One pass: the water, the map's terrain and details keyed from 0 up, the details
		// layer's own (region names, footprints), the map's details keyed below 0, the
		// entities and the effects, the fog of war where its mask has it, then the labels.
		let water = self
			.textures
			.slot(WATER_TEXTURE)
			.and_then(Slot::texture)
			.and_then(|water| self.water.prepare(gpu, view, water, scene.extent, self.clock.virtual_time, ambient));
		let fog = self
			.textures
			.slot(FOG_TEXTURE)
			.and_then(Slot::texture)
			.filter(|_| scene.fog_of_war)
			.and_then(|fog| self.fog.prepare(gpu, view, fog, scene.extent, self.clock.real, ambient));
		let (before, after) = (self.kept.calls(&shown, true), self.kept.calls(&shown, false));
		let moving_calls: [Vec<_>; kept_sprites::PARTS] = std::array::from_fn(|part| match world {
			None => self.moving.calls(part),
			Some(_) => Vec::new(),
		});
		let [details, map, labels] = &frame.parts;
		let parts = match world {
			Some(_) => [Part::Kept(&before), Part::Streamed(details), Part::Kept(&after), Part::Streamed(map), Part::Streamed(labels)],
			None => [
				Part::Kept(&before),
				Part::Instanced(&moving_calls[0]),
				Part::Kept(&after),
				Part::Instanced(&moving_calls[1]),
				Part::Instanced(&moving_calls[2]),
			],
		};
		let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		let prepared: Prepared = self.sprites.prepare(gpu, &mut encoder, &uniforms, &parts, &ready);
		{
			let mut pass = begin_pass(&mut encoder, target, Some(CLEAR), "world");
			if let Some(water) = &water {
				water.record(&mut pass);
			}
			self.sprites.record(&mut pass, &prepared, &parts, 0..4);
			if let Some(fog) = &fog {
				fog.record(&mut pass);
			}
			self.sprites.record(&mut pass, &prepared, &parts, 4..5);
		}
		self.sprites.finish(&encoder);
		gpu.queue.submit([encoder.finish()]);
		drop(sprites_span);
		drop(ready);
		drop(moving_calls);
		self.frame = frame;
		if let Some(world) = world {
			self.quad_scratch.recycle(world);
		}
		let _span = crate::frame_timing::span("fit budget");
		self.fit_budget();
		stats
	}

	/// Send this frame's asks for frames to the pages' loader and wait for them (a
	/// frame that waits for all loads them now): whether any were asked for.
	fn ask(&mut self, gpu: &Gpu, wait: bool) -> bool {
		match self.pages.as_mut() {
			Some(p) if wait => {
				p.load_asks_now(gpu, &self.mips);
				true
			}
			Some(p) => {
				let asked = p.send_asks();
				if !asked.is_empty() {
					p.pump_until(gpu, &self.mips, &asked, std::time::Instant::now() + crate::pages::WAIT_BUDGET);
				}
				!asked.is_empty()
			}
			None => false,
		}
	}

	/// The atlases the moving layer draws whole this frame: loaded (or waited for), drawn
	/// now, given their mips if drawn at `zoom` samples them, and their usage recorded if
	/// tracked. Returns how many failed to load.
	fn moving_atlases(&mut self, gpu: &Gpu, atlases: Atlases, zoom: f32) -> usize {
		let upload = |path: &str, image: bb_assets::Image| upload(gpu, path, &image);
		if self.usage.is_some() {
			let mut list = crate::world::AtlasList::default();
			let quads = self
				.moving
				.drawn_quads()
				.map(|(path, quad)| Quad {
					texture: list.index_of(path),
					..*quad
				})
				.collect();
			let map = crate::world::WorldQuads {
				quads,
				atlases: list,
				labels_from: 0,
				details_end: 0,
			};
			self.record_usage(&map, zoom, None);
		}
		let mut failed = 0;
		for atlas in self.moving.drawn_from(self.pages.as_mut()) {
			let path = self.moving.atlas_paths()[atlas].clone();
			if path == WHITE || glyph_page(&path).is_some() {
				continue;
			}
			match atlases {
				Atlases::WaitForAll => {
					let _ = self.textures.get_now(&path, upload);
				}
				Atlases::DrawReady => {
					let _ = self.textures.get_or_upload(&path, upload);
				}
			}
			self.residency.drawn(&path, Drawer::World);
			if matches!(self.textures.slot(&path), Some(Slot::Missing(..) | Slot::Failed(_))) {
				failed += 1;
			}
			let Some(size) = self
				.textures
				.slot(&path)
				.and_then(Slot::texture)
				.filter(|t| t.texture.mip_level_count() == 1 && !path.starts_with(FONT_DIR))
				.map(|t| (t.width, t.height))
			else {
				continue;
			};
			if self.moving.whole_quads(atlas).any(|quad| samples_mips(quad, zoom, size, None)) {
				let (mips, textures) = (&self.mips, &mut self.textures);
				textures.update(&path, |texture| Some(mips.with_mips(gpu, texture)));
			}
		}
		failed
	}

	/// The atlases the kept map's segments `shown` draw whole: loaded (or waited for),
	/// drawn now, given their mips if drawn at `zoom` samples them, and their usage
	/// recorded if tracked. Returns how many failed to load, and the quads drawn from
	/// those not ready (or failed with no stand-in), which draw nothing.
	fn kept_atlases(&mut self, gpu: &Gpu, shown: &kept_map::Shown, atlases: Atlases, zoom: f32) -> (usize, usize) {
		let upload = |path: &str, image: bb_assets::Image| upload(gpu, path, &image);
		if self.usage.is_some() {
			let mut list = crate::world::AtlasList::default();
			for path in self.kept.atlas_paths() {
				list.index_of(path);
			}
			let map = crate::world::WorldQuads {
				quads: self.kept.shown_quads(shown).copied().collect(),
				atlases: list,
				labels_from: 0,
				details_end: 0,
			};
			self.record_usage(&map, zoom, None);
		}
		let (mut failed, mut unready) = (0, 0);
		for atlas in self.kept.drawn_from(shown, self.pages.as_mut()) {
			let path = self.kept.atlas_paths()[atlas].clone();
			match atlases {
				Atlases::WaitForAll => {
					let _ = self.textures.get_now(&path, upload);
				}
				Atlases::DrawReady => {
					let _ = self.textures.get_or_upload(&path, upload);
				}
			}
			self.residency.drawn(&path, Drawer::World);
			if matches!(self.textures.slot(&path), Some(Slot::Missing(..) | Slot::Failed(_))) {
				failed += 1;
			}
			if self.textures.slot(&path).and_then(Slot::texture).is_none() {
				unready += self.kept.whole_quads(shown, atlas).count();
			}
			let Some(size) = self
				.textures
				.slot(&path)
				.and_then(Slot::texture)
				.filter(|t| t.texture.mip_level_count() == 1 && !path.starts_with(FONT_DIR))
				.map(|t| (t.width, t.height))
			else {
				continue;
			};
			if self.kept.whole_quads(shown, atlas).any(|quad| samples_mips(quad, zoom, size, None)) {
				let (mips, textures) = (&self.mips, &mut self.textures);
				textures.update(&path, |texture| Some(mips.with_mips(gpu, texture)));
			}
		}
		(failed, unready)
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
		let map = {
			let _span = crate::frame_timing::span("battle quads");
			tactical_quads(scene, registry, view)
		};
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

	/// Draw a battle's orientation overlays over what `target` holds, `fit` target
	/// pixels to a pixel of the game's screen (see [`crate::orientation`]). An atlas
	/// still loading draws nothing (with `Atlases::DrawReady`).
	pub fn draw_orientation(
		&mut self,
		gpu: &Gpu,
		target: &RenderTarget,
		registry: &SpriteRegistry,
		overlays: &[squirrel_bb::scene::tactical::SceneOrientationOverlay],
		fit: f32,
		atlases: Atlases,
	) -> FrameStats {
		let Some(map) = crate::orientation::orientation_quads(registry, overlays, fit) else {
			return FrameStats::default();
		};
		let uniforms = SceneUniforms::centered([0.0, 0.0], target.width as f32, target.height as f32);
		self.record_usage(&map, 1.0, None);
		self.index(registry);
		self.draw_quads(gpu, target, &map, &uniforms, None, atlases, Drawer::Battle, (1.0, None))
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
		let mut frame = std::mem::take(&mut self.frame);
		let resolve_span = crate::frame_timing::span("resolve+bind");
		self.resolve(gpu, map, atlases, drawer, (zoom, dof_below), &mut frame.resolved);
		let (ready, mut stats) = self.bind(map, &frame.resolved, &[], &mut frame.parts[..1]);
		drop(resolve_span);
		stats.quads = frame.parts[0].len();
		let _span = crate::frame_timing::span("sprites draw");
		self.sprites.draw(gpu, target, uniforms, clear, &frame.parts[0], &ready);
		drop(ready);
		self.frame = frame;
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
