//! The world map's moving layer kept on the GPU from frame to frame: entities' sprites
//! and labels, clouds, region names, footprints, waves, particles and birds, around the
//! map `crate::kept_map` keeps.
//!
//! Each quad is an instance of the sprite program (`crate::sprite::instances`): a
//! record on the GPU, written when it's first drawn and again only when it changes. A
//! frame:
//!
//! - **Matches the scene's items with the last frame's** ([`crate::world_items`]), kind
//!   by kind in push order. An item equal to the one kept at its place (or a few places
//!   on: items come and go) keeps its quads and their records; any other is built
//!   anew, in new places. A moving party's sprite, a drifting cloud or a particle is
//!   built again each frame it moves; a still map builds nothing.
//! - **Sorts what the view shows** on the CPU, as the per-frame build does: by layer,
//!   then the higher key first, ties in push order, the items culled as before. The
//!   order goes to the GPU as a list of instance places (4 bytes each), uploaded only
//!   when it differs from the last frame's.
//! - **Locates** a quad drawn in our pages once (`crate::pages`), and again when the
//!   pages' generation moves on; batches the order in calls of up to eight atlases,
//!   whose bind groups are kept.
//!
//! The quads, their order and their texture coordinates are the per-frame build's, and
//! the instances draw them through the same fragment stage, so the frame is the same
//! (`world_pan --compare`).

use std::ops::Range;

use bb_assets::SpriteRegistry;
use squirrel_bb::scene::WorldScene;

use crate::fast_hash::FastMap;
use crate::fonts::shaper::glyph_page;
use crate::labels::WHITE;
use crate::pages::{FramePages, Located};
use crate::residency::Drawer;
use crate::sprite::instances::SLOT_BITS;
use crate::sprite::{Quad, SLOTS, SpriteRenderer};
use crate::world::{AtlasList, Layer, WorldFonts, WorldView, sort_back_to_front};
use crate::world_items::{ItemQuad, Item, KINDS, items};

/// How many places on an item is looked for among the last frame's when it isn't at
/// its own (items gone in between).
const LOOK_AHEAD: usize = 8;

/// Where a kept quad is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum From {
	/// The renderer's white texel (label plates).
	White,
	/// One of the glyph pages.
	Glyph(usize),
	/// One of our pages.
	Page(usize),
	/// Its atlas, whole: the kept atlas at this index.
	Whole(usize),
}

/// What an atlas is drawn as.
#[derive(Clone, Copy)]
enum AtlasKind {
	White,
	Glyph(usize),
	/// From our pages, unless they give it up as whole.
	Paged,
	Whole,
}

/// What the items were built with: the registry's sheets and the fonts the renderer has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Built {
	pub sheets: usize,
	pub fonts: usize,
}

/// An item kept, and the places of its quads.
struct Kept {
	item: Item,
	at: usize,
	len: usize,
}

/// A kept quad: its layer, sort key, atlas (in [`KeptSprites::atlases`]) and quad in its
/// atlas's coordinates, and where it was found, if it was at the pages' generation then.
struct KeptQuad {
	layer: Layer,
	key: i64,
	atlas: usize,
	quad: Quad,
	located: Option<(u64, From, Quad)>,
}

/// The parts of a frame's order, drawn in turn: the details layer's own (region names,
/// footprints), the entities and effects, the labels.
pub(crate) const PARTS: usize = 3;

/// The moving layer kept on the GPU.
#[derive(Default)]
pub(crate) struct KeptSprites {
	built: Option<Built>,
	/// Each kind's items, in push order.
	kinds: [Vec<Kept>; KINDS],
	/// Each instance place's quad (`None`: free).
	quads: Vec<Option<KeptQuad>>,
	/// The free places, by the length of the run.
	free: FastMap<usize, Vec<usize>>,
	atlases: AtlasList,
	kinds_of: Vec<AtlasKind>,
	/// Whether the atlases are drawn from pages.
	paged: bool,
	/// The places the view draws, in drawing order.
	order: Vec<u32>,
	keys: Vec<u64>,
	drawn: Vec<u32>,
	/// Where each frame was found this frame, by atlas and texture coordinates.
	memo: FastMap<(usize, [u32; 8]), Located>,
	generation: u64,
	/// The order list uploaded last, and this frame's calls: each part's, a range of the
	/// list and the slots' sources.
	uploaded: Vec<u32>,
	entries: Vec<u32>,
	calls: [Vec<(Range<u32>, Vec<From>)>; PARTS],
	/// The sort keys of the first part's entries (the details layer's, which come first
	/// in the order list), for drawing them among the kept map's.
	detail_keys: Vec<i64>,
	/// Bind groups by the sources they bind, with the views they were made with.
	groups: FastMap<Vec<From>, (Vec<wgpu::TextureView>, wgpu::BindGroup)>,
	/// Per kind during a frame's matching: the next kept item, and once one didn't match,
	/// the rest of the last frame's (taken as matched) and the next of them.
	cursors: [usize; KINDS],
	tails: [Option<(Vec<Option<Kept>>, usize)>; KINDS],
}

/// What locating quads takes.
pub(crate) struct Locating<'a> {
	pub pages: Option<&'a mut FramePages>,
	/// Load what's missing now (a frame that waits for all).
	pub wait: bool,
	pub generation: u64,
}

/// What a frame drew of the moving layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Drawn {
	pub quads: usize,
	/// Waiting for their frame or atlas.
	pub waiting: usize,
}

impl KeptSprites {
	/// Bring the kept items up to `scene`'s at `view`, built with `sprites` and `fonts`
	/// (`built` says what they are; anything else drops what was kept), with atlases
	/// drawn from pages if `paged`; and sort what the view shows.
	pub(crate) fn update<'a>(
		&mut self,
		scene: &WorldScene,
		view: &WorldView,
		sprites: &'a SpriteRegistry,
		fonts: WorldFonts<'a>,
		built: Built,
		paged: bool,
	) {
		if self.built != Some(built) || self.paged != paged {
			*self = KeptSprites {
				built: Some(built),
				paged,
				..KeptSprites::default()
			};
		}
		self.cursors = [0; KINDS];
		let mut scratch = Vec::new();
		for item in items(scene, view, fonts) {
			self.offer(item, sprites, fonts, &mut scratch);
		}
		for kind in 0..KINDS {
			let gone: Vec<Kept> = match self.tails[kind].take() {
				Some((tail, _)) => tail.into_iter().flatten().collect(),
				None => {
					let at = self.cursors[kind];
					self.kinds[kind].split_off(at)
				}
			};
			for kept in gone {
				self.release(kept);
			}
		}
		self.sort(view);
	}

	/// Take the frame's next `item`: keep the last frame's equal one, or build it.
	fn offer<'a>(&mut self, item: Item, sprites: &'a SpriteRegistry, fonts: WorldFonts<'a>, scratch: &mut Vec<ItemQuad<'a>>) {
		let kind = item.kind();
		if self.tails[kind].is_none() {
			let at = self.cursors[kind];
			if self.kinds[kind].get(at).is_some_and(|kept| kept.item == item) {
				self.cursors[kind] += 1;
				return;
			}
			// From here on, the last frame's are matched as they come.
			let tail = self.kinds[kind].split_off(at).into_iter().map(Some).collect();
			self.tails[kind] = Some((tail, 0));
		}
		let (tail, next) = self.tails[kind].as_mut().expect("a tail");
		let end = (*next + LOOK_AHEAD).min(tail.len());
		if let Some(found) = (*next..end).find(|&i| tail[i].as_ref().is_some_and(|kept| kept.item == item)) {
			let passed: Vec<Kept> = tail[*next..found].iter_mut().filter_map(Option::take).collect();
			let kept = tail[found].take().expect("found");
			*next = found + 1;
			for gone in passed {
				self.release(gone);
			}
			self.kinds[kind].push(kept);
			return;
		}
		scratch.clear();
		item.quads(sprites, fonts, scratch);
		let len = scratch.len();
		let at = self.place(len);
		for (i, q) in scratch.drain(..).enumerate() {
			let atlas = self.atlas(q.texture);
			self.quads[at + i] = Some(KeptQuad {
				layer: q.layer,
				key: q.key,
				atlas,
				quad: q.quad,
				located: None,
			});
		}
		self.kinds[kind].push(Kept { item, at, len });
	}

	/// Places for a run of `len` quads.
	fn place(&mut self, len: usize) -> usize {
		if len == 0 {
			return 0;
		}
		if let Some(at) = self.free.get_mut(&len).and_then(Vec::pop) {
			return at;
		}
		let at = self.quads.len();
		self.quads.resize_with(at + len, || None);
		at
	}

	/// `kept` is gone: its places are free.
	fn release(&mut self, kept: Kept) {
		if kept.len == 0 {
			return;
		}
		for q in &mut self.quads[kept.at..kept.at + kept.len] {
			*q = None;
		}
		self.free.entry(kept.len).or_default().push(kept.at);
	}

	/// The kept atlas id of `path`.
	fn atlas(&mut self, path: &str) -> usize {
		let id = self.atlases.index_of(path);
		while self.kinds_of.len() < self.atlases.paths.len() {
			let path = &self.atlases.paths[self.kinds_of.len()];
			self.kinds_of.push(if path == WHITE {
				AtlasKind::White
			} else if let Some(page) = glyph_page(path) {
				AtlasKind::Glyph(page)
			} else if self.paged && !path.starts_with(crate::world_renderer::FONT_DIR) {
				AtlasKind::Paged
			} else {
				AtlasKind::Whole
			});
		}
		id
	}

	/// The places `view` draws, back to front: by layer, then the higher key first,
	/// ties in push order (kind by kind, item by item, quad by quad).
	fn sort(&mut self, view: &WorldView) {
		self.drawn.clear();
		for kept in self.kinds.iter().flatten().filter(|kept| kept.item.shows(view)) {
			self.drawn.extend((kept.at..kept.at + kept.len).map(|at| at as u32));
		}
		let (quads, drawn) = (&self.quads, &self.drawn);
		sort_back_to_front(
			drawn.len(),
			|i| {
				let q = quads[drawn[i] as usize].as_ref().expect("a kept quad");
				(q.layer, q.key)
			},
			&mut self.keys,
			&mut self.order,
		);
		for i in &mut self.order {
			*i = drawn[*i as usize];
		}
	}

	/// The instances' places: how many (for the room kept on the GPU).
	pub(crate) fn places(&self) -> usize {
		self.quads.len()
	}

	/// The instances' room was made anew: every quad is written again when next drawn.
	pub(crate) fn unwritten(&mut self) {
		for q in self.quads.iter_mut().flatten() {
			q.located = None;
		}
	}

	/// A new frame at the pages' `generation`: where frames lie is asked again, and if
	/// the generation moved on, the bind groups go (they may hold pages gone).
	pub(crate) fn begin_frame(&mut self, generation: u64) {
		self.memo.clear();
		if generation != self.generation {
			self.generation = generation;
			self.groups.clear();
		}
	}

	/// Drop every bind group (the textures they hold may be gone).
	pub(crate) fn forget_groups(&mut self) {
		self.groups.clear();
	}

	/// Frames that came in since this frame asked: their quads are located again.
	pub(crate) fn frames_came(&mut self) {
		self.memo.retain(|_, at| !matches!(at, Located::Waiting));
	}

	/// Locate the drawn quads not located at `cx`'s generation, writing each found to its
	/// instance in `renderer`; how many wait for their frame.
	pub(crate) fn locate(&mut self, cx: &mut Locating, renderer: &SpriteRenderer) -> usize {
		let mut waiting = 0;
		for &at in &self.order {
			let q = self.quads[at as usize].as_mut().expect("a kept quad");
			if q.located.as_ref().is_some_and(|(generation, ..)| *generation == cx.generation) {
				continue;
			}
			let found = match self.kinds_of[q.atlas] {
				AtlasKind::White => Some((From::White, q.quad)),
				AtlasKind::Glyph(page) => Some((From::Glyph(page), q.quad)),
				AtlasKind::Whole => Some((From::Whole(q.atlas), q.quad)),
				AtlasKind::Paged => {
					let pages = cx.pages.as_deref_mut().expect("paged");
					let path = &self.atlases.paths[q.atlas];
					let key = (q.atlas, uv_bits(&q.quad.uv));
					let at = *self
						.memo
						.entry(key)
						.or_insert_with(|| pages.locate(path, &q.quad, Drawer::World, cx.wait));
					match at {
						Located::Page { page, map } => Some((
							From::Page(page),
							Quad {
								uv: map.uv(q.quad.uv),
								..q.quad
							},
						)),
						Located::Waiting => None,
						Located::Whole => Some((From::Whole(q.atlas), q.quad)),
					}
				}
			};
			match found {
				Some((from, quad)) => {
					renderer.write_instance(at as usize, &quad);
					q.located = Some((cx.generation, from, quad));
				}
				None => {
					q.located = None;
					waiting += 1;
				}
			}
		}
		waiting
	}

	/// What the drawn quads are drawn from: the pages (marked drawn this frame in
	/// `pages`) and the atlases drawn whole, by their kept id.
	pub(crate) fn drawn_from(&self, pages: Option<&mut FramePages>) -> Vec<usize> {
		let mut whole = Vec::new();
		let mut touched = Vec::new();
		for &at in &self.order {
			match self.quads[at as usize].as_ref().and_then(|q| q.located.as_ref()) {
				Some((_, From::Whole(atlas), _)) if !whole.contains(atlas) => whole.push(*atlas),
				Some((_, From::Page(page), _)) if !touched.contains(page) => touched.push(*page),
				_ => {}
			}
		}
		if let Some(pages) = pages {
			for page in touched {
				pages.touch_page(page);
			}
		}
		whole
	}

	/// The kept atlases' paths.
	pub(crate) fn atlas_paths(&self) -> &[String] {
		&self.atlases.paths
	}

	/// The drawn quads located whole in atlas `atlas` (to see if they sample its mips).
	pub(crate) fn whole_quads(&self, atlas: usize) -> impl Iterator<Item = &Quad> + '_ {
		self.order.iter().filter_map(move |&at| match self.quads[at as usize].as_ref()?.located.as_ref()? {
			(_, From::Whole(a), quad) if *a == atlas => Some(quad),
			_ => None,
		})
	}

	/// The drawn quads in their atlases' coordinates, each with its atlas's path (usage
	/// tracking).
	pub(crate) fn drawn_quads(&self) -> impl Iterator<Item = (&str, &Quad)> + '_ {
		self.order.iter().filter_map(move |&at| {
			let q = self.quads[at as usize].as_ref()?;
			Some((self.atlases.paths[q.atlas].as_str(), &q.quad))
		})
	}

	/// Batch the drawn quads that are located and whose source `view_of` has ready into
	/// calls of up to eight sources, part by part, each call's bind group made (or kept)
	/// in `renderer`; upload the order list if it changed.
	pub(crate) fn bind<'v>(
		&mut self,
		renderer: &SpriteRenderer,
		gpu: &crate::Gpu,
		view_of: impl Fn(From) -> Option<&'v wgpu::TextureView>,
	) -> Drawn {
		let mut drawn = Drawn::default();
		self.entries.clear();
		self.detail_keys.clear();
		for calls in &mut self.calls {
			calls.clear();
		}
		let mut ready: FastMap<From, bool> = FastMap::default();
		for &at in &self.order {
			let q = self.quads[at as usize].as_ref().expect("a kept quad");
			let Some((_, from, _)) = q.located else {
				drawn.waiting += 1;
				continue;
			};
			if !*ready.entry(from).or_insert_with(|| view_of(from).is_some()) {
				drawn.waiting += 1;
				continue;
			}
			let part = match q.layer {
				Layer::Labels => 2,
				Layer::Details | Layer::Terrain => 0,
				Layer::Entities | Layer::Effects => 1,
			};
			if part == 0 {
				self.detail_keys.push(q.key);
			}
			let entry = self.entries.len() as u32;
			let calls = &mut self.calls[part];
			let fits = calls.last().is_some_and(|(range, slots)| range.end == entry && (slots.contains(&from) || slots.len() < SLOTS));
			if !fits {
				calls.push((entry..entry, Vec::new()));
			}
			let (range, slots) = calls.last_mut().expect("a call");
			let slot = match slots.iter().position(|f| *f == from) {
				Some(slot) => slot,
				None => {
					slots.push(from);
					slots.len() - 1
				}
			};
			range.end = entry + 1;
			self.entries.push(at << SLOT_BITS | slot as u32);
			drawn.quads += 1;
		}
		if self.entries != self.uploaded {
			renderer.set_instance_order(&self.entries);
			self.uploaded.clone_from(&self.entries);
		}
		for (_, slots) in self.calls.iter().flatten() {
			let views: Vec<&wgpu::TextureView> = slots.iter().map(|f| view_of(*f).expect("ready")).collect();
			let same = self
				.groups
				.get(slots)
				.is_some_and(|(held, _)| held.len() == views.len() && held.iter().zip(&views).all(|(a, b)| a == *b));
			if !same {
				let group = renderer.views_group(gpu, &views);
				self.groups.insert(slots.clone(), (views.into_iter().cloned().collect(), group));
			}
		}
		drawn
	}

	/// The sort keys of the first part's quads, in its order (the order list's first
	/// entries).
	pub(crate) fn detail_keys(&self) -> &[i64] {
		&self.detail_keys
	}

	/// Part `part`'s calls (see [`PARTS`]): each a range of the order list and its bind
	/// group.
	pub(crate) fn calls(&self, part: usize) -> Vec<(Range<u32>, &wgpu::BindGroup)> {
		self.calls[part]
			.iter()
			.map(|(range, slots)| (range.clone(), &self.groups[slots].1))
			.collect()
	}
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_sprites::registry;
	use squirrel_bb::scene::{SceneEntity, SceneFootprint, SceneSprite};

	fn party(id: i64, x: f64) -> SceneEntity {
		SceneEntity {
			id,
			pos: (x, 0.0),
			shown: true,
			in_details: false,
			key: 0,
			sprites: vec![SceneSprite {
				name: "body".into(),
				brush: "unit".into(),
				color: 0xffff_ffff,
				scale: 1.0,
				visible: true,
				offset: (0.0, 0.0),
				flipped: false,
				rotation: 0.0,
				ignore_ambient: false,
				scaling: true,
				colorization: true,
				frame: 0,
			}],
			labels: Vec::new(),
		}
	}

	fn step(x: f32) -> SceneFootprint {
		SceneFootprint { brush: "step".into(), pos: (x, 0.0), scale: 1.0, alpha: 255.0 }
	}

	/// Each item's places, kind by kind.
	fn places(kept: &KeptSprites) -> Vec<Vec<usize>> {
		kept.kinds.iter().map(|kind| kind.iter().map(|k| k.at).collect()).collect()
	}

	/// An item that didn't change keeps its quads where they are, so nothing is written
	/// for it again; one that moved is built again, and those around one gone keep theirs.
	#[test]
	fn items_that_stay_the_same_keep_their_places() {
		let registry = registry();
		let view = WorldView { fit: 1.0, center: [0.0, 0.0], zoom: 1.0, width: 800, height: 600 };
		let built = Built { sheets: 1, fonts: 0 };
		let mut kept = KeptSprites::default();
		let mut scene = WorldScene {
			entities: vec![party(1, 0.0), party(2, 10.0), party(3, 20.0)],
			footprints: vec![step(1.0), step(2.0), step(3.0)],
			..WorldScene::default()
		};
		kept.update(&scene, &view, &registry, WorldFonts::default(), built, false);
		let first = places(&kept);
		assert_eq!(kept.order.len(), 6);
		kept.update(&scene, &view, &registry, WorldFonts::default(), built, false);
		assert_eq!(places(&kept), first, "a still frame keeps every place");
		assert_eq!(kept.places(), 6, "and makes none");

		// The second party moves; the first footprint goes.
		scene.entities[1].pos.0 += 1.0;
		scene.footprints.remove(0);
		kept.update(&scene, &view, &registry, WorldFonts::default(), built, false);
		let now = places(&kept);
		assert_eq!((now[0][0], now[0][2]), (first[0][0], first[0][2]), "the still parties keep theirs");
		assert_ne!(now[0][1], first[0][1], "the moved one is built again");
		assert_eq!(now[3], first[3][1..], "the footprints after the one gone keep theirs");
		assert_eq!(kept.order.len(), 5);
	}
}
