//! Our own pages of frames (see [`crate::residency`]; bindless is a possible fast
//! path over them later). The GPU holds the frames the draws use rather than whole
//! vanilla atlases: each frame's region
//! (`bb_assets::frames`: the frame with its atlas neighbours around it, on 8-texel
//! boundaries) placed in one of our 2048² pages. A quad is drawn from its frame's place
//! ([`FramePages::locate`]); a frame not on the GPU yet is asked of the frame loader
//! (our cached sheets, `bb_assets::SheetCache`) and its quads wait.
//!
//! - On a first run the cache lacks the sheets: every atlas's is built ahead on spare
//!   workers, after the game's own loads (`bb_assets::FrameLoader::warm_up`; about a
//!   second for vanilla's 26), and logged in one line.
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
//!   until its page is empty). After a battle's end or the campaign's, the frames left
//!   are packed again into fewer pages if they fit ([`FramePages::compact`]): the
//!   battle's holes don't keep the world's pages.
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
	/// A page's side ([`PAGE`]; smaller in tests).
	size: u32,
	alloc: PageAlloc,
	textures: Vec<Option<GpuTexture>>,
	/// The last frame each page was drawn from.
	used: Vec<u64>,
}

impl Pages {
	fn new(size: u32) -> Pages {
		Pages {
			size,
			alloc: PageAlloc::new(size),
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
#[derive(Clone, Copy)]
pub(crate) enum Located {
	/// A page; its texture coordinates are the atlas's mapped by `map`.
	Page { page: usize, map: PageMap },
	/// Its frame is on its way.
	Waiting,
	/// Not the pages' to draw: its atlas stays a whole texture.
	Whole,
}

/// How an atlas's texture coordinates map into the page holding its frame.
#[derive(Clone, Copy)]
pub(crate) struct PageMap {
	/// The atlas's size, the frame's shift from its place in the atlas to its place in the
	/// page, and the page's size, in texels.
	atlas: (f32, f32),
	shift: (f32, f32),
	page: (u32, u32),
}

impl PageMap {
	/// `uv` (the atlas's) in the page.
	pub(crate) fn uv(&self, uv: [[f32; 2]; 4]) -> [[f32; 2]; 4] {
		let ((aw, ah), (dx, dy), (pw, ph)) = (self.atlas, self.shift, self.page);
		uv.map(|[u, v]| [(u * aw + dx) / pw as f32, (v * ah + dy) / ph as f32])
	}
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
	/// Frames went at a discard: pack the rest again before the next draw.
	compact_due: bool,
	frame: u64,
	/// Counts the times a frame left its place or an atlas went whole (see
	/// [`FramePages::generation`]).
	generation: u64,
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
		Self::with_page_size(source, cache, threads, PAGE)
	}

	fn with_page_size(source: Arc<dyn FileSource>, cache: Option<SheetCache>, threads: usize, page: u32) -> FramePages {
		FramePages {
			loader: FrameLoader::new(source.clone(), cache.clone(), threads),
			source,
			cache,
			atlases: HashMap::new(),
			indexed: 0,
			pages: Pages::new(page),
			resident: HashMap::new(),
			pending: HashSet::new(),
			asks: HashMap::new(),
			whole: HashSet::new(),
			warmed: HashSet::new(),
			compact_due: false,
			frame: 0,
			generation: 0,
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
		// A first run's cache lacks the sheets: they're built ahead, on spare workers,
		// so the frames come as small reads when first drawn (see `FrameLoader::warm_up`).
		if self.cache.is_some() {
			let requests = self
				.atlases
				.iter()
				.map(|(atlas, index)| FrameRequest {
					atlas: atlas.clone(),
					frames: index.frames.clone(),
					wanted: Vec::new(),
				})
				.collect();
			self.loader.warm_up(requests);
		}
	}

	pub(crate) fn begin_frame(&mut self, frame: u64) {
		self.frame = frame;
	}

	/// Changes when a frame that was placed leaves its place (evicted, or moved by a
	/// compaction) or an atlas goes whole: what was located before may be elsewhere now.
	/// A frame coming in changes nothing that was located.
	pub(crate) fn generation(&self) -> u64 {
		self.generation
	}

	/// Page `page` is drawn from this frame (as [`FramePages::locate`] marks it): quads
	/// located before, and kept, draw from it.
	pub(crate) fn touch_page(&mut self, page: usize) {
		if let Some(used) = self.pages.used.get_mut(page) {
			*used = self.frame;
		}
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
		let page = self.pages.alloc.page_size(resident.place.page).expect("a page in use");
		let map = PageMap {
			atlas: (size.0 as f32, size.1 as f32),
			shift: (resident.place.x as f32 - resident.region.x as f32, resident.place.y as f32 - resident.region.y as f32),
			page,
		};
		Located::Page {
			page: resident.place.page,
			map,
		}
	}

	/// Send this frame's asks to the loader, those not on their way already, split
	/// across the workers ([`ASK_CHUNK`] frames a request); the frames asked for.
	pub(crate) fn send_asks(&mut self) -> Vec<(String, TexelRect)> {
		let mut sent = Vec::new();
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
			}
			sent.extend(wanted.into_iter().map(|f| (atlas.clone(), f)));
		}
		sent
	}

	/// Take in the loader's answers until the frames `asked` for are in (or failed), or
	/// `deadline` passed.
	pub(crate) fn pump_until(&mut self, gpu: &Gpu, mips: &MipMaker, asked: &[(String, TexelRect)], deadline: Instant) {
		while asked.iter().any(|key| self.pending.contains(key)) {
			let Some((atlas, loaded)) = self.loader.take_until(deadline) else { return };
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
	/// Take in the frames the loader finished, after packing the pages again if a
	/// discard asked for it.
	pub(crate) fn pump(&mut self, gpu: &Gpu, mips: &MipMaker) {
		if std::mem::take(&mut self.compact_due) {
			self.compact(gpu, mips);
		}
		while let Some((atlas, loaded)) = self.loader.try_take() {
			self.take_in(gpu, mips, atlas, loaded);
		}
	}

	/// Pack the frames into fewer pages if they fit in fewer: each frame's region moves
	/// on the GPU, its first [`LEVELS`] levels copied and the rest made again from them,
	/// as [`FramePages::put`] makes them. Regions go tallest first, which packs shelves
	/// tighter than the order they came in.
	fn compact(&mut self, gpu: &Gpu, mips: &MipMaker) {
		let mut keys: Vec<(String, TexelRect)> = self.resident.keys().cloned().collect();
		keys.sort_by(|a, b| {
			let (ra, rb) = (self.resident[a].region, self.resident[b].region);
			(rb.h, rb.w).cmp(&(ra.h, ra.w)).then_with(|| a.cmp(b))
		});
		let mut trial = PageAlloc::new(self.pages.size);
		for key in &keys {
			let region = self.resident[key].region;
			trial.alloc(region.w, region.h);
		}
		if trial.pages().count() >= self.pages.alloc.pages().count() {
			return;
		}
		self.generation += 1;
		let mut fresh = Pages::new(self.pages.size);
		let moves: Vec<Place> = keys.iter().map(|key| fresh.place(gpu, self.resident[key].region)).collect();
		let mut encoder = gpu
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("compact pages") });
		for (key, to) in keys.iter().zip(&moves) {
			let r = &self.resident[key];
			let (from, into) = (&self.pages.texture(r.place.page).texture, &fresh.texture(to.page).texture);
			let levels = (LEVELS as u32).min(from.mip_level_count()).min(into.mip_level_count());
			for level in 0..levels {
				let at = |x: u32, y: u32| wgpu::Origin3d { x: x >> level, y: y >> level, z: 0 };
				encoder.copy_texture_to_texture(
					wgpu::TexelCopyTextureInfo {
						texture: from,
						mip_level: level,
						origin: at(r.place.x, r.place.y),
						aspect: wgpu::TextureAspect::All,
					},
					wgpu::TexelCopyTextureInfo {
						texture: into,
						mip_level: level,
						origin: at(to.x, to.y),
						aspect: wgpu::TextureAspect::All,
					},
					wgpu::Extent3d {
						width: (r.region.w >> level).max(1),
						height: (r.region.h >> level).max(1),
						depth_or_array_layers: 1,
					},
				);
			}
		}
		gpu.submit("gpu page moves", encoder.finish());
		for (key, to) in keys.iter().zip(moves) {
			let r = self.resident.get_mut(key).expect("a resident frame");
			mips.remake_region(gpu, &fresh.texture(to.page).texture, (to.x, to.y, r.region.w, r.region.h), LEVELS as u32);
			// Last drawn when its frame or its page was (quads kept from frame to frame
			// mark only their pages, see `touch_page`).
			fresh.used[to.page] = fresh.used[to.page].max(r.frame).max(self.pages.used[r.place.page]);
			r.place = to;
		}
		self.pages = fresh;
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
				self.generation += 1;
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
			self.generation += 1;
			self.pages.free(r.place);
		}
	}

	/// A battle ended: the frames only battles drew go.
	pub(crate) fn battle_discarded(&mut self) {
		let battle = bit(Drawer::Battle);
		let gone: Vec<_> = self.resident.iter().filter(|(_, r)| r.drawers == battle).map(|(k, _)| k.clone()).collect();
		self.compact_due |= !gone.is_empty();
		for key in gone {
			self.evict(&key);
		}
	}

	/// The campaign was left: every frame this frame didn't draw goes.
	pub(crate) fn campaign_discarded(&mut self) {
		let frame = self.frame;
		let gone: Vec<_> = self.resident.iter().filter(|(_, r)| r.frame < frame).map(|(k, _)| k.clone()).collect();
		self.compact_due |= !gone.is_empty();
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

#[cfg(test)]
mod tests {
	use super::*;
	use bb_assets::frames::cut_levels;
	use bb_assets::mips::mip_levels;
	use bb_assets::{BrushFrame, BrushSheet, BrushSprite, Image, SheetHeader, name_hash};

	const W: u32 = 64;
	const H: u32 = 40;
	const ATLAS: &str = "gfx/t.png";

	fn atlas() -> Image {
		Image {
			width: W,
			height: H,
			rgba: (0..W * H * 4).map(|i| (i * 37 % 251) as u8).collect(),
		}
	}

	struct Files(Vec<u8>);

	impl FileSource for Files {
		fn read(&self, path: &str) -> Option<Vec<u8>> {
			(path == ATLAS).then(|| self.0.clone())
		}
	}

	fn png(image: &Image) -> Vec<u8> {
		let mut out = Vec::new();
		let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
		encoder.set_color(png::ColorType::Rgba);
		encoder.set_depth(png::BitDepth::Eight);
		let mut writer = encoder.write_header().unwrap();
		writer.write_image_data(&image.rgba).unwrap();
		writer.finish().unwrap();
		out
	}

	/// Eight 8×8 frames, four to a row, two rows: each one's region is 24×24.
	fn frames() -> Vec<TexelRect> {
		(0..2).flat_map(|j| (0..4).map(move |i| TexelRect { x: 8 + 16 * i, y: 8 + 16 * j, w: 8, h: 8 })).collect()
	}

	fn uv(f: &TexelRect) -> [f32; 4] {
		[f.x as f32 / W as f32, (f.x + f.w) as f32 / W as f32, f.y as f32 / H as f32, (f.y + f.h) as f32 / H as f32]
	}

	fn registry() -> SpriteRegistry {
		let sprites = frames()
			.iter()
			.enumerate()
			.map(|(i, f)| {
				let name = format!("f{i}");
				BrushSprite {
					hash: name_hash(name.as_bytes()),
					name,
					material: 0,
					width: 8,
					height: 8,
					offset_x: 0,
					offset_y: 0,
					depth_bias: 0,
					frame_time_ms: 100,
					unknown_pause: [0, 0],
					rotation_speed: 0.0,
					flags: 0x403,
					unknown_offset: [0.0, 0.0],
					color: [255; 4],
					unknown_scale: 1.0,
					frames: vec![BrushFrame {
						name: String::new(),
						uv: uv(f),
						rect: [-4.0, 4.0, -4.0, 4.0],
					}],
					trailer: [0; 3],
				}
			})
			.collect::<Vec<_>>();
		let mut registry = SpriteRegistry::new();
		registry.add_sheet(
			"brushes/t.brush",
			BrushSheet {
				texture: ATLAS.into(),
				header: SheetHeader {
					width: W as u16,
					height: H as u16,
					flags: [0; 12],
					sprite_count: sprites.len() as u32,
					reserved: [0; 2],
					max_level: 1000,
				},
				sprites,
			},
		);
		registry
	}

	fn quad(f: &TexelRect) -> Quad {
		let [u0, u1, v0, v1] = uv(f);
		Quad {
			texture: 0,
			corners: [[0.0; 2]; 4],
			uv: [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
			color: [255; 4],
			corner_colors: None,
			overlay: [0; 4],
			factor: 0.0,
			saturation: 1.0,
			flags: 0,
			level: 0,
		}
	}

	/// After a battle's frames go, the world's frames left in half-empty pages are packed
	/// into fewer, every level they keep the same as their atlas's.
	#[test]
	fn a_battles_holes_are_packed_away() {
		let gpu = match Gpu::headless() {
			Ok(gpu) => gpu,
			Err(e) => {
				bb_engine::harness::skip(&e);
				return;
			}
		};
		let mips = MipMaker::new(&gpu);
		let mut pages = FramePages::with_page_size(Arc::new(Files(png(&atlas()))), None, 1, 48);
		pages.index(&registry());
		let frames = frames();
		for f in &frames {
			assert!(matches!(pages.locate(ATLAS, &quad(f), Drawer::World, true), Located::Waiting));
		}
		pages.load_asks_now(&gpu, &mips);
		// Every other frame only a battle draws: each page keeps two of the world's.
		for (i, f) in frames.iter().enumerate() {
			let drawer = if i % 2 == 0 { Drawer::World } else { Drawer::Battle };
			pages.resident.get_mut(&(ATLAS.to_string(), *f)).unwrap().drawers = 0;
			assert!(matches!(pages.locate(ATLAS, &quad(f), drawer, false), Located::Page { .. }));
		}
		assert_eq!(pages.memory().0, 2, "two pages, four frames each");

		pages.battle_discarded();
		assert_eq!(pages.memory().0, 2, "the battle's frames leave holes");
		pages.pump(&gpu, &mips);
		assert_eq!((pages.memory().0, pages.memory().1), (1, 4), "packed into one page");

		let chain = mip_levels(&atlas(), LEVELS);
		for level in 0..2u32 {
			let page = crate::target::read_level(&gpu, &pages.texture(0).texture, level).expect("read back");
			for (i, f) in frames.iter().enumerate().filter(|(i, _)| i % 2 == 0) {
				let r = &pages.resident[&(ATLAS.to_string(), *f)];
				let want = &cut_levels(&chain, r.region)[level as usize];
				let (x0, y0) = (r.place.x >> level, r.place.y >> level);
				for y in 0..want.height {
					let at = (((y0 + y) * page.width + x0) * 4) as usize;
					let row = (y * want.width * 4) as usize;
					assert_eq!(
						&page.rgba[at..at + (want.width * 4) as usize],
						&want.rgba[row..row + (want.width * 4) as usize],
						"frame {i}, level {level}, row {y}"
					);
				}
			}
		}
	}
}
