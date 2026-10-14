//! Our own pages of frames (see [`crate::residency`]; bindless is a possible fast
//! path over them later). The GPU holds the frames the draws use rather than whole
//! vanilla atlases: each frame's region
//! (`bb_assets::frames`: the frame with its atlas neighbours around it, on 8-texel
//! boundaries) placed in one of our 2048² pages. A quad is drawn from its frame's place
//! ([`FramePages::locate`]); a frame not on the GPU yet is asked of the frame loader
//! (our cached sheets, `bb_assets::SheetCache`) and its quads wait.
//!
//! - A frame that asks waits for the answers up to [`WAIT_BUDGET`], so what comes into
//!   view (a camera glide's edge, a jump's whole screen) is drawn in the frame it
//!   appears, as from whole atlases, when the sheets are cached; past the budget (an
//!   atlas's sheet still being built) its sprites follow a frame or more later. An
//!   animated sprite's frames come together (`bb_assets::frames::animations`).
//! - A frame comes with its first mip levels, cut from its atlas's own (see
//!   `bb_assets::frames::cut_levels`), so a page samples the atlas's texels at every
//!   level the game reads; the levels past them are made on the GPU from the last.
//! - Residency as the atlases' (see [`crate::residency`]), per frame: what only battles
//!   drew goes at a battle's end, all of it when the campaign is left; over the budget,
//!   whole pages go, the least recently used first (a frame freed alone frees no memory
//!   until its page is empty).
//! - Atlases the pages don't take stay whole textures: the fonts, the water and the fog,
//!   an atlas without brushes, a quad that isn't one of its atlas's frames, an atlas that
//!   fails to load (its "texture missing" stand-in).

pub(crate) mod alloc;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_assets::frames::{LEVELS, Region, TexelRect, animations, atlas_frames};
use bb_assets::{FileSource, FrameLoader, FrameRequest, LoadedFrame, SheetCache, SpriteRegistry};

use self::alloc::{PageAlloc, Place};
use crate::gpu::Gpu;
use crate::mips::MipMaker;
use crate::residency::Drawer;
use crate::sprite::Quad;
use crate::texture::{GpuTexture, TEXTURE_FORMAT, mip_count, texture_bytes};

/// A page's side, in texels.
pub const PAGE: u32 = 2048;

/// How long a frame that asked for frames waits for them: half a 60 Hz frame, beside
/// our own few milliseconds of work (under vsync the thread would wait for the present
/// anyway). A screenful from cached sheets takes a few milliseconds across the workers.
pub const WAIT_BUDGET: Duration = Duration::from_millis(8);

/// The most frames one request to the loader asks for, so a draw's asks spread over
/// the workers.
const ASK_CHUNK: usize = 8;

/// An atlas's frames.
struct AtlasFrames {
	size: (u32, u32),
	frames: Arc<[TexelRect]>,
	set: HashSet<TexelRect>,
	/// An animated sprite's frames, from each of them (see `bb_assets::frames::animations`).
	animations: HashMap<TexelRect, Arc<[TexelRect]>>,
}

/// The pages.
struct Pages {
	alloc: PageAlloc,
	textures: Vec<Option<GpuTexture>>,
	/// The last frame each page was drawn from.
	used: Vec<u64>,
}

impl Pages {
	fn new() -> Pages {
		Pages {
			alloc: PageAlloc::new(PAGE),
			textures: Vec::new(),
			used: Vec::new(),
		}
	}

	/// A place for `region`, its page made if new.
	fn place(&mut self, gpu: &Gpu, region: Region) -> Place {
		let (place, new) = self.alloc.alloc(region.w, region.h);
		if self.textures.len() <= place.page {
			self.textures.resize_with(place.page + 1, || None);
			self.used.resize(place.page + 1, 0);
		}
		if new {
			let (width, height) = self.alloc.page_size(place.page).expect("just made");
			let levels = mip_count(width, height);
			let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
				label: Some("page"),
				size: wgpu::Extent3d {
					width,
					height,
					depth_or_array_layers: 1,
				},
				mip_level_count: levels,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: TEXTURE_FORMAT,
				usage: wgpu::TextureUsages::TEXTURE_BINDING
					| wgpu::TextureUsages::COPY_DST
					| wgpu::TextureUsages::COPY_SRC
					| wgpu::TextureUsages::RENDER_ATTACHMENT,
				view_formats: &[],
			});
			let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
			self.textures[place.page] = Some(GpuTexture { texture, view, width, height });
		}
		place
	}

	/// Give `place` back; its page goes if it's now empty.
	fn free(&mut self, place: Place) {
		if self.alloc.free(place) {
			self.textures[place.page] = None;
		}
	}

	fn texture(&self, page: usize) -> &GpuTexture {
		self.textures[page].as_ref().expect("a page in use")
	}

	fn bytes(&self) -> u64 {
		self.textures.iter().flatten().map(|t| texture_bytes(&t.texture)).sum()
	}
}

/// A frame on the GPU.
struct Resident {
	place: Place,
	region: Region,
	/// The last frame drawn, and who ever drew it (`Drawer` bits).
	frame: u64,
	drawers: u8,
}

/// Where a quad is drawn from.
pub(crate) enum Located {
	/// A page, at these texture coordinates.
	Page { page: usize, uv: [[f32; 2]; 4] },
	/// Its frame is on its way.
	Waiting,
	/// Not the pages' to draw: its atlas stays a whole texture.
	Whole,
}

/// Our pages, the frames in them, and the loads on their way.
pub(crate) struct FramePages {
	source: Arc<dyn FileSource>,
	cache: Option<SheetCache>,
	loader: FrameLoader,
	atlases: HashMap<String, AtlasFrames>,
	indexed: usize,
	pages: Pages,
	resident: HashMap<(String, TexelRect), Resident>,
	/// Frames asked for and not loaded yet.
	pending: HashSet<(String, TexelRect)>,
	/// This frame's asks, by atlas.
	asks: HashMap<String, Vec<TexelRect>>,
	/// Atlases that failed to load as frames: drawn whole.
	whole: HashSet<String>,
	/// Atlases warmed (their sheets made ready on a worker).
	warmed: HashSet<String>,
	frame: u64,
}

fn bit(drawer: Drawer) -> u8 {
	match drawer {
		Drawer::World => 1,
		Drawer::Battle => 2,
		Drawer::Other => 4,
	}
}

/// The texel rectangle `uv` spans in an atlas of `size`.
fn uv_rect(uv: &[[f32; 2]; 4], (width, height): (u32, u32)) -> TexelRect {
	let span = |i: usize| uv.iter().fold((f32::MAX, f32::MIN), |(lo, hi), c| (lo.min(c[i]), hi.max(c[i])));
	let ((u0, u1), (v0, v1)) = (span(0), span(1));
	TexelRect::of_uv([u0, u1, v0, v1], width, height)
}

impl FramePages {
	pub(crate) fn new(source: Arc<dyn FileSource>, cache: Option<SheetCache>, threads: usize) -> FramePages {
		FramePages {
			loader: FrameLoader::new(source.clone(), cache.clone(), threads),
			source,
			cache,
			atlases: HashMap::new(),
			indexed: 0,
			pages: Pages::new(),
			resident: HashMap::new(),
			pending: HashSet::new(),
			asks: HashMap::new(),
			whole: HashSet::new(),
			warmed: HashSet::new(),
			frame: 0,
		}
	}

	/// A script hinted at `atlas`: its sheet is made ready on a worker (built from the
	/// atlas if the cache lacks it, its index read otherwise), so its frames load as
	/// soon as a draw asks. Whether the pages draw it (if not, it's warmed whole).
	pub(crate) fn warm(&mut self, atlas: &str) -> bool {
		if self.whole.contains(atlas) {
			return false;
		}
		let Some(index) = self.atlases.get(atlas) else { return false };
		if self.warmed.insert(atlas.to_string()) {
			self.loader.request(FrameRequest {
				atlas: atlas.to_string(),
				frames: index.frames.clone(),
				wanted: Vec::new(),
			});
		}
		true
	}

	/// Index the atlases' frames from `registry` (again if it changed).
	pub(crate) fn index(&mut self, registry: &SpriteRegistry) {
		if registry.sheets().len() == self.indexed {
			return;
		}
		self.indexed = registry.sheets().len();
		let sizes: HashMap<String, (u32, u32)> = registry
			.sheets()
			.iter()
			.map(|s| (s.sheet.texture.clone(), (u32::from(s.sheet.header.width), u32::from(s.sheet.header.height))))
			.collect();
		let mut animations = animations(registry);
		self.atlases = atlas_frames(registry)
			.into_iter()
			.filter_map(|(path, frames)| {
				let size = *sizes.get(&path)?;
				let set = frames.iter().copied().collect();
				let animations = animations.remove(&path).unwrap_or_default();
				Some((
					path,
					AtlasFrames {
						size,
						frames: frames.into(),
						set,
						animations,
					},
				))
			})
			.collect();
	}

	pub(crate) fn begin_frame(&mut self, frame: u64) {
		self.frame = frame;
	}

	/// Where to draw `quad` of `atlas` from for `drawer`: its frame's place; or waiting
	/// (its frame asked for, for this thread to load if `wait`); or whole.
	pub(crate) fn locate(&mut self, atlas: &str, quad: &Quad, drawer: Drawer, wait: bool) -> Located {
		if self.whole.contains(atlas) {
			return Located::Whole;
		}
		let Some(index) = self.atlases.get(atlas) else { return Located::Whole };
		let size = index.size;
		let rect = uv_rect(&quad.uv, size);
		let frame = if index.set.contains(&rect) {
			rect
		} else {
			match index.frames.iter().find(|f| f.contains(&rect)) {
				Some(f) => *f,
				None => return Located::Whole,
			}
		};
		let key = (atlas.to_string(), frame);
		let now = self.frame;
		let Some(resident) = self.resident.get_mut(&key) else {
			// Asked for once (again, if a draw that can't wait needs it now), with the
			// rest of its animation.
			let group = index.animations.get(&frame).cloned();
			let frames = group.as_deref().unwrap_or(std::slice::from_ref(&frame));
			for &f in frames {
				let key = (atlas.to_string(), f);
				if self.resident.contains_key(&key) {
					continue;
				}
				if self.pending.insert(key) || (wait && f == frame) {
					let asks = self.asks.entry(atlas.to_string()).or_default();
					if !asks.contains(&f) {
						asks.push(f);
					}
				}
			}
			return Located::Waiting;
		};
		resident.frame = now;
		resident.drawers |= bit(drawer);
		self.pages.used[resident.place.page] = now;
		let (pw, ph) = self.pages.alloc.page_size(resident.place.page).expect("a page in use");
		let (aw, ah) = (size.0 as f32, size.1 as f32);
		let (dx, dy) = (resident.place.x as f32 - resident.region.x as f32, resident.place.y as f32 - resident.region.y as f32);
		let uv = quad.uv.map(|[u, v]| [(u * aw + dx) / pw as f32, (v * ah + dy) / ph as f32]);
		Located::Page {
			page: resident.place.page,
			uv,
		}
	}

	/// Send this frame's asks to the loader, those not on their way already, split
	/// across the workers ([`ASK_CHUNK`] frames a request); how many requests went.
	pub(crate) fn send_asks(&mut self) -> usize {
		let mut sent = 0;
		for (atlas, frames) in std::mem::take(&mut self.asks) {
			let Some(index) = self.atlases.get(&atlas) else { continue };
			let wanted: Vec<TexelRect> = frames
				.into_iter()
				.filter(|f| !self.resident.contains_key(&(atlas.clone(), *f)))
				.collect();
			for chunk in wanted.chunks(ASK_CHUNK) {
				self.loader.request(FrameRequest {
					atlas: atlas.clone(),
					frames: index.frames.clone(),
					wanted: chunk.to_vec(),
				});
				sent += 1;
			}
		}
		sent
	}

	/// Take in the loader's answers until `count` with frames (or failures) came, or
	/// `deadline` passed. A warm-up's answer (no frames) doesn't count.
	pub(crate) fn pump_until(&mut self, gpu: &Gpu, mips: &MipMaker, count: usize, deadline: Instant) {
		let mut left = count;
		while left > 0 {
			let Some((atlas, loaded)) = self.loader.take_until(deadline) else { return };
			if loaded.as_ref().map_or(true, |frames| !frames.is_empty()) {
				left -= 1;
			}
			self.take_in(gpu, mips, atlas, loaded);
		}
	}

	/// Load this frame's asks on this thread (a frame for a screenshot or a portrait).
	pub(crate) fn load_asks_now(&mut self, gpu: &Gpu, mips: &MipMaker) {
		for (atlas, wanted) in std::mem::take(&mut self.asks) {
			let Some(index) = self.atlases.get(&atlas) else { continue };
			let request = FrameRequest {
				atlas: atlas.clone(),
				frames: index.frames.clone(),
				wanted,
			};
			let loaded = self.loader.load_now(&*self.source, self.cache.as_ref(), &request);
			self.take_in(gpu, mips, atlas, loaded);
		}
	}

	/// Take in the frames the loader finished.
	pub(crate) fn pump(&mut self, gpu: &Gpu, mips: &MipMaker) {
		while let Some((atlas, loaded)) = self.loader.try_take() {
			self.take_in(gpu, mips, atlas, loaded);
		}
	}

	fn take_in(&mut self, gpu: &Gpu, mips: &MipMaker, atlas: String, loaded: Result<Vec<LoadedFrame>, String>) {
		match loaded {
			Ok(frames) => {
				for frame in frames {
					let key = (atlas.clone(), frame.frame);
					self.pending.remove(&key);
					if self.resident.contains_key(&key) {
						continue;
					}
					self.put(gpu, mips, key, frame);
				}
			}
			Err(_) => {
				// Drawn whole from now on: the atlas's own loading (and its "texture
				// missing" stand-in) takes over.
				self.pending.retain(|(a, _)| *a != atlas);
				self.whole.insert(atlas);
			}
		}
	}

	/// Put a loaded frame in a page: its levels where they go, and the levels past them
	/// made again over its place.
	fn put(&mut self, gpu: &Gpu, mips: &MipMaker, key: (String, TexelRect), frame: LoadedFrame) {
		let place = self.pages.place(gpu, frame.region);
		let texture = &self.pages.texture(place.page).texture;
		for (level, image) in frame.levels.iter().enumerate().take(texture.mip_level_count() as usize) {
			gpu.queue.write_texture(
				wgpu::TexelCopyTextureInfo {
					texture,
					mip_level: level as u32,
					origin: wgpu::Origin3d {
						x: place.x >> level,
						y: place.y >> level,
						z: 0,
					},
					aspect: wgpu::TextureAspect::All,
				},
				&image.rgba,
				wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(image.width * 4),
					rows_per_image: Some(image.height),
				},
				wgpu::Extent3d {
					width: image.width,
					height: image.height,
					depth_or_array_layers: 1,
				},
			);
		}
		mips.remake_region(gpu, texture, (place.x, place.y, frame.region.w, frame.region.h), LEVELS as u32);
		self.pages.used[place.page] = self.frame;
		self.resident.insert(
			key,
			Resident {
				place,
				region: frame.region,
				frame: self.frame,
				drawers: 0,
			},
		);
	}

	/// The page's texture to bind for a [`Located::Page`].
	pub(crate) fn texture(&self, page: usize) -> &GpuTexture {
		self.pages.texture(page)
	}

	fn evict(&mut self, key: &(String, TexelRect)) {
		if let Some(r) = self.resident.remove(key) {
			self.pages.free(r.place);
		}
	}

	/// A battle ended: the frames only battles drew go.
	pub(crate) fn battle_discarded(&mut self) {
		let battle = bit(Drawer::Battle);
		let gone: Vec<_> = self.resident.iter().filter(|(_, r)| r.drawers == battle).map(|(k, _)| k.clone()).collect();
		for key in gone {
			self.evict(&key);
		}
	}

	/// The campaign was left: every frame this frame didn't draw goes.
	pub(crate) fn campaign_discarded(&mut self) {
		let frame = self.frame;
		let gone: Vec<_> = self.resident.iter().filter(|(_, r)| r.frame < frame).map(|(k, _)| k.clone()).collect();
		for key in gone {
			self.evict(&key);
		}
	}

	/// The pages' GPU bytes.
	pub(crate) fn bytes(&self) -> u64 {
		self.pages.bytes()
	}

	/// Drop whole pages, the least recently used first (never one this frame drew from),
	/// until the pages take no more than `budget` bytes.
	pub(crate) fn fit(&mut self, budget: u64) {
		while self.bytes() > budget {
			let oldest = self
				.pages
				.alloc
				.pages()
				.map(|page| (page, self.pages.used[page]))
				.filter(|(_, used)| *used < self.frame)
				.min_by_key(|(_, used)| *used);
			let Some((page, _)) = oldest else { return };
			let gone: Vec<_> = self
				.resident
				.iter()
				.filter(|(_, r)| r.place.page == page)
				.map(|(k, _)| k.clone())
				.collect();
			for key in gone {
				self.evict(&key);
			}
		}
	}

	/// What the pages hold: how many pages and frames, and their bytes.
	pub(crate) fn memory(&self) -> (usize, usize, u64) {
		let pages = self.pages.alloc.pages().count();
		(pages, self.resident.len(), self.bytes())
	}
}
