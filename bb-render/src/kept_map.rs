//! The world map's terrain and details, kept on the GPU from frame to frame.
//!
//! Zoomed out, the world map is ~32,500 sprites, nearly all of them tiles' terrain and
//! details that don't change from one frame to the next. They're built once, in the
//! order they draw, written to a vertex buffer in world units (the camera and the day's
//! light are uniforms), and drawn from it every frame; only what changed is built again:
//!
//! - The map's order changes (a terrain brush or mirroring set, details spawned or
//!   cleared, the map sized, cleared or loaded): everything is built again
//!   (`squirrel_bb::WorldGrid::map_revision`), as the sprite registry's sheets changing.
//! - A detail's look changes (a script set its colour, brush, offset, …, or it turned,
//!   stepped or faded on its own): only its quad is written again
//!   (`WorldGrid::take_detail_changes`). Its place in the order stays: a detail's key is
//!   its spawn's, and a hidden one, or one whose brush is unknown, keeps an empty quad.
//! - Where a frame lies in our pages changes (a page evicted or packed again; see
//!   `FramePages::generation`): each part of the map is located again when next drawn.
//!
//! The order is the original's, as [`crate::world::world_quads`] builds it: terrain,
//! then the details back to front by their key (with the spawn tile's axial-r parity),
//! ties in spawn order. What the details layer draws besides the map (region names and
//! footprints, key 0, after the map's own key-0 details) goes between the map's details
//! with keys ≥ 0 and those below ([`KeptMap::split`]).
//!
//! The map is cut into segments of [`SEGMENT`] consecutive quads of that order. A
//! segment is drawn if its quads' bounds meet the screen, located in the pages when it's
//! first drawn (a frame then loads as it would have), and drawn in its own calls of up to
//! eight atlases, so skipping one keeps the order of the rest. Its bounds are its
//! quads' own corners, so what it skips is off the screen.

use std::ops::Range;

use bb_assets::SpriteRegistry;
use squirrel_bb::scene::{SceneDetail, SceneTile};

use crate::fast_hash::FastMap;
use crate::pages::{FramePages, Located};
use crate::residency::Drawer;
use crate::sprite::{Quad, SLOTS, SpriteRenderer};
use crate::world::{
	AtlasIds, AtlasList, Layer, WorldView, detail_quad, sort_back_to_front, terrain_quad,
};

/// Quads in a segment.
pub(crate) const SEGMENT: usize = 256;

/// What the kept map was built from: the map's revision and the registry's sheets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Built {
	pub revision: u64,
	pub sheets: usize,
}

/// Where a kept quad is drawn from (see `crate::world_renderer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum From {
	/// Its atlas, whole: the kept map's atlas at this index.
	Whole(usize),
	/// One of our pages.
	Page(usize),
}

/// The map's terrain and details in drawing order.
#[derive(Default)]
pub(crate) struct KeptMap {
	/// `None`: built for one frame only (a scene given whole, see `WorldRenderer::draw`).
	built: Option<Built>,
	/// Each quad, in its atlas's coordinates, its `texture` indexing `atlases`; `None`
	/// draws nothing (hidden, or an unknown brush).
	items: Vec<Option<Quad>>,
	atlases: AtlasList,
	/// Each detail's quad, by its id.
	ids: FastMap<i64, usize>,
	split: usize,
	segments: Vec<Segment>,
	/// Where each frame was found this frame, by atlas and texture coordinates.
	memo: FastMap<(usize, [u32; 8]), Located>,
	/// The pages' generation the segments' bind groups were made at.
	generation: u64,
}

struct Segment {
	items: Range<usize>,
	/// The quads' extent, `[left, right, bottom, top]` (left > right with none).
	bounds: [f32; 4],
	placed: Option<Placed>,
	/// Quads changed since it was placed.
	patched: Vec<usize>,
}

/// A segment located in the pages and written to the kept buffer.
struct Placed {
	generation: u64,
	calls: Vec<KeptCall>,
	/// The pages it draws from, and the atlases it draws whole.
	pages: Vec<usize>,
	whole: Vec<usize>,
	/// Quads drawn, and those waiting for their frame.
	drawn: Vec<bool>,
	waiting: usize,
}

struct KeptCall {
	items: Range<usize>,
	slots: Vec<From>,
	/// Its bind group, and the views it was made with.
	group: Option<(Vec<wgpu::TextureView>, wgpu::BindGroup)>,
}

/// What locating a segment takes.
pub(crate) struct Placing<'a> {
	/// Our pages (`None`: every atlas is drawn whole).
	pub pages: Option<&'a mut FramePages>,
	/// Load what's missing now (a frame that waits for all).
	pub wait: bool,
	pub sprites: &'a SpriteRenderer,
	/// Whether an atlas path is drawn whole even with pages (fonts).
	pub whole: &'a dyn Fn(&str) -> bool,
}

/// A quad located for drawing.
enum At {
	Nothing,
	Waiting,
	Drawn(From, Quad),
}

/// A frame's segments to draw, and what they drew.
#[derive(Default)]
pub(crate) struct Shown {
	pub segments: Vec<usize>,
	pub quads: usize,
	pub waiting: usize,
}

fn uv_bits(uv: &[[f32; 2]; 4]) -> [u32; 8] {
	let mut out = [0; 8];
	for (i, [u, v]) in uv.iter().enumerate() {
		out[2 * i] = u.to_bits();
		out[2 * i + 1] = v.to_bits();
	}
	out
}

/// `bounds` grown to hold `quad`'s corners.
fn grow(bounds: &mut [f32; 4], quad: &Quad) {
	for [x, y] in quad.corners {
		bounds[0] = bounds[0].min(x);
		bounds[1] = bounds[1].max(x);
		bounds[2] = bounds[2].min(y);
		bounds[3] = bounds[3].max(y);
	}
}

const EMPTY: [f32; 4] = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];

impl KeptMap {
	/// Whether it must be built again for `built` (always for `None`).
	pub(crate) fn stale(&self, built: Option<Built>) -> bool {
		built.is_none() || self.built != built
	}

	/// Build it from `tiles` (the whole map, or for `built` `None` what one frame shows).
	pub(crate) fn build(
		&mut self,
		tiles: &[SceneTile],
		sprites: &SpriteRegistry,
		built: Option<Built>,
	) {
		struct Entry {
			layer: Layer,
			key: i64,
			id: Option<i64>,
			quad: Option<(usize, Quad)>,
		}
		let mut ids = AtlasIds::default();
		let mut entries = Vec::new();
		for tile in tiles {
			if let Some((key, texture, quad)) = terrain_quad(tile, sprites) {
				entries.push(Entry {
					layer: Layer::Terrain,
					key,
					id: None,
					quad: Some((ids.id(texture), quad)),
				});
			}
			for detail in &tile.details {
				let quad = (!detail.hidden)
					.then(|| detail_quad(detail, sprites))
					.flatten();
				entries.push(Entry {
					layer: Layer::Details,
					key: i64::from(detail.z),
					id: Some(detail.id),
					quad: quad.map(|(texture, quad)| (ids.id(texture), quad)),
				});
			}
		}
		let (mut keys, mut order) = (Vec::new(), Vec::new());
		sort_back_to_front(
			entries.len(),
			|i| (entries[i].layer, entries[i].key),
			&mut keys,
			&mut order,
		);
		let mut atlases = AtlasList::default();
		let mut index = vec![usize::MAX; ids.paths.len()];
		self.ids.clear();
		self.items.clear();
		self.split = order.len();
		for (at, &i) in order.iter().enumerate() {
			let entry = &entries[i as usize];
			if let Some(id) = entry.id {
				self.ids.insert(id, at);
			}
			if entry.layer == Layer::Details && entry.key < 0 && self.split == order.len() {
				self.split = at;
			}
			self.items.push(entry.quad.map(|(texture, quad)| {
				let atlas = &mut index[texture];
				if *atlas == usize::MAX {
					*atlas = atlases.index_of(ids.paths[texture]);
				}
				Quad {
					texture: *atlas,
					..quad
				}
			}));
		}
		self.atlases = atlases;
		self.segments.clear();
		for part in [0..self.split, self.split..self.items.len()] {
			let mut start = part.start;
			while start < part.end {
				let items = start..(start + SEGMENT).min(part.end);
				let mut bounds = EMPTY;
				for quad in self.items[items.clone()].iter().flatten() {
					grow(&mut bounds, quad);
				}
				self.segments.push(Segment {
					items: items.clone(),
					bounds,
					placed: None,
					patched: Vec::new(),
				});
				start = items.end;
			}
		}
		self.built = built;
	}

	/// The kept quads: how many.
	pub(crate) fn len(&self) -> usize {
		self.items.len()
	}

	/// The kept buffer was made anew: every segment is written again.
	pub(crate) fn unplace(&mut self) {
		for segment in &mut self.segments {
			segment.placed = None;
		}
	}

	/// Take the details whose look changed: their quads, built again.
	pub(crate) fn change(&mut self, changes: &[SceneDetail], sprites: &SpriteRegistry) {
		for detail in changes {
			let Some(&at) = self.ids.get(&detail.id) else {
				continue;
			};
			let quad = (!detail.hidden)
				.then(|| detail_quad(detail, sprites))
				.flatten();
			let quad = quad.map(|(texture, quad)| Quad {
				texture: self.atlases.index_of(texture),
				..quad
			});
			if quad == self.items[at] {
				continue;
			}
			self.items[at] = quad;
			// Segments are in order, SEGMENT quads each but where the split cuts one short.
			let segment = self.segment_of(at);
			let segment = &mut self.segments[segment];
			if let Some(quad) = &quad {
				grow(&mut segment.bounds, quad);
			}
			segment.patched.push(at);
		}
	}

	fn segment_of(&self, item: usize) -> usize {
		self.segments.partition_point(|s| s.items.end <= item)
	}

	/// The segments `view` shows, in order.
	pub(crate) fn shown(&self, view: &WorldView) -> Shown {
		let [left, right, bottom, top] = view.bounds(0.0);
		Shown {
			segments: (0..self.segments.len())
				.filter(|&i| {
					let [l, r, b, t] = self.segments[i].bounds;
					l <= right && r >= left && b <= top && t >= bottom
				})
				.collect(),
			..Shown::default()
		}
	}

	/// A new frame, at the pages' `generation`: where frames lie is asked again, and if
	/// the generation moved on, each segment's bind groups go (they may hold pages gone).
	pub(crate) fn begin_frame(&mut self, generation: u64) {
		self.memo.clear();
		self.follow_generation(generation);
	}

	/// The pages are at `generation`: if it moved on, the bind groups go (they may hold
	/// pages gone, which would stay on the GPU while held).
	pub(crate) fn follow_generation(&mut self, generation: u64) {
		if generation != self.generation {
			self.generation = generation;
			self.forget_groups();
		}
	}

	/// Drop every bind group (the textures they hold may be gone).
	pub(crate) fn forget_groups(&mut self) {
		for call in self
			.segments
			.iter_mut()
			.filter_map(|s| s.placed.as_mut())
			.flat_map(|p| &mut p.calls)
		{
			call.group = None;
		}
	}

	/// Frames that came in since this frame asked: their quads are located again.
	pub(crate) fn frames_came(&mut self) {
		self.memo.retain(|_, at| !matches!(at, Located::Waiting));
	}

	/// Locate the segments `shown` that need it at pages `generation` and write them:
	/// those not placed yet, placed at an earlier generation or still waiting for frames;
	/// a segment with quads changed has only those written, unless one needs an atlas
	/// its call lacks room for. Counts what they draw into `shown`.
	pub(crate) fn place(&mut self, shown: &mut Shown, generation: u64, cx: &mut Placing) {
		shown.quads = 0;
		shown.waiting = 0;
		for &s in &shown.segments {
			let fresh = self.segments[s]
				.placed
				.as_ref()
				.is_some_and(|p| p.generation == generation && p.waiting == 0);
			if !(fresh && self.patch(s, cx)) {
				self.place_segment(s, generation, cx);
			}
			let placed = self.segments[s].placed.as_ref().expect("placed");
			shown.quads += placed.drawn.iter().filter(|d| **d).count();
			shown.waiting += placed.waiting;
		}
	}

	/// Where quad `i` is drawn from.
	fn locate(&mut self, i: usize, cx: &mut Placing) -> At {
		let Some(quad) = self.items[i] else {
			return At::Nothing;
		};
		let path = &self.atlases.paths[quad.texture];
		if let Some(pages) = cx.pages.as_deref_mut()
			&& !(cx.whole)(path)
		{
			let key = (quad.texture, uv_bits(&quad.uv));
			let at = *self
				.memo
				.entry(key)
				.or_insert_with(|| pages.locate(path, &quad, Drawer::World, cx.wait));
			match at {
				Located::Page { page, map } => {
					return At::Drawn(
						From::Page(page),
						Quad {
							uv: map.uv(quad.uv),
							..quad
						},
					);
				}
				Located::Waiting => return At::Waiting,
				Located::Whole => {}
			}
		}
		At::Drawn(From::Whole(quad.texture), quad)
	}

	/// Write segment `s`'s changed quads into its calls; `false` if it must be placed
	/// whole again (a frame waiting, or no room for a new atlas).
	fn patch(&mut self, s: usize, cx: &mut Placing) -> bool {
		let mut patched = std::mem::take(&mut self.segments[s].patched);
		patched.sort_unstable();
		patched.dedup();
		let start = self.segments[s].items.start;
		let mut writes = Vec::new();
		for &i in &patched {
			let at = self.locate(i, cx);
			let placed = self.segments[s].placed.as_mut().expect("placed");
			let call = placed
				.calls
				.iter_mut()
				.find(|c| c.items.contains(&i))
				.expect("a call holds every quad");
			let drawn = match at {
				At::Waiting => return false,
				At::Nothing => None,
				At::Drawn(from, quad) => {
					let slot = match call.slots.iter().position(|f| *f == from) {
						Some(slot) => slot,
						None if call.slots.len() < SLOTS => {
							call.slots.push(from);
							call.group = None;
							match from {
								From::Page(page) if !placed.pages.contains(&page) => {
									placed.pages.push(page)
								}
								From::Whole(atlas) if !placed.whole.contains(&atlas) => {
									placed.whole.push(atlas)
								}
								_ => {}
							}
							call.slots.len() - 1
						}
						None => return false,
					};
					Some((quad, slot as u32))
				}
			};
			placed.drawn[i - start] = drawn.is_some();
			writes.push((i, drawn));
		}
		for (i, drawn) in writes {
			cx.sprites.write_kept(i, [drawn]);
		}
		true
	}

	/// Locate segment `s` whole, batch it in calls and write it.
	fn place_segment(&mut self, s: usize, generation: u64, cx: &mut Placing) {
		let items = self.segments[s].items.clone();
		self.segments[s].patched.clear();
		let mut placed = Placed {
			generation,
			calls: vec![KeptCall {
				items: items.start..items.start,
				slots: Vec::new(),
				group: None,
			}],
			pages: Vec::new(),
			whole: Vec::new(),
			drawn: Vec::with_capacity(items.len()),
			waiting: 0,
		};
		let mut writes = Vec::with_capacity(items.len());
		for i in items.clone() {
			let drawn = match self.locate(i, cx) {
				At::Nothing => None,
				At::Waiting => {
					placed.waiting += 1;
					None
				}
				At::Drawn(from, quad) => {
					let call = placed.calls.last_mut().expect("a call");
					let slot = match call.slots.iter().position(|f| *f == from) {
						Some(slot) => slot,
						None => {
							if call.slots.len() == SLOTS {
								placed.calls.push(KeptCall {
									items: i..i,
									slots: Vec::new(),
									group: None,
								});
							}
							let call = placed.calls.last_mut().expect("a call");
							call.slots.push(from);
							match from {
								From::Page(page) if !placed.pages.contains(&page) => {
									placed.pages.push(page)
								}
								From::Whole(atlas) if !placed.whole.contains(&atlas) => {
									placed.whole.push(atlas)
								}
								_ => {}
							}
							call.slots.len() - 1
						}
					};
					Some((quad, slot as u32))
				}
			};
			placed.drawn.push(drawn.is_some());
			placed.calls.last_mut().expect("a call").items.end = i + 1;
			writes.push(drawn);
		}
		cx.sprites.write_kept(items.start, writes);
		self.segments[s].placed = Some(placed);
	}

	/// The atlases the segments `shown` draw whole (by path), and mark the pages they draw
	/// from as drawn this frame.
	pub(crate) fn drawn_from(
		&self,
		shown: &Shown,
		mut pages: Option<&mut FramePages>,
	) -> Vec<usize> {
		let mut whole = Vec::new();
		for placed in shown
			.segments
			.iter()
			.filter_map(|&s| self.segments[s].placed.as_ref())
		{
			if let Some(pages) = pages.as_deref_mut() {
				for &page in &placed.pages {
					pages.touch_page(page);
				}
			}
			for &atlas in &placed.whole {
				if !whole.contains(&atlas) {
					whole.push(atlas);
				}
			}
		}
		whole
	}

	/// The kept atlases' paths.
	pub(crate) fn atlas_paths(&self) -> &[String] {
		&self.atlases.paths
	}

	/// The quads drawn whole of atlas `atlas` in the segments `shown` (to see if they
	/// sample its mips).
	pub(crate) fn whole_quads<'a>(
		&'a self,
		shown: &'a Shown,
		atlas: usize,
	) -> impl Iterator<Item = &'a Quad> + 'a {
		shown
			.segments
			.iter()
			.flat_map(|&s| self.items[self.segments[s].items.clone()].iter().flatten())
			.filter(move |q| q.texture == atlas)
	}

	/// The quads of the segments `shown`, in their atlases' coordinates (usage tracking).
	pub(crate) fn shown_quads<'a>(
		&'a self,
		shown: &'a Shown,
	) -> impl Iterator<Item = &'a Quad> + 'a {
		shown
			.segments
			.iter()
			.flat_map(|&s| self.items[self.segments[s].items.clone()].iter().flatten())
	}

	/// Give each call of the segments `shown` its bind group, made again if the views its
	/// atlases are drawn from (`view_of`) changed.
	pub(crate) fn bind<'v>(
		&mut self,
		shown: &Shown,
		sprites: &SpriteRenderer,
		gpu: &crate::Gpu,
		view_of: impl Fn(From) -> &'v wgpu::TextureView,
	) {
		for &s in &shown.segments {
			let Some(placed) = self.segments[s].placed.as_mut() else {
				continue;
			};
			for call in &mut placed.calls {
				let views: Vec<&wgpu::TextureView> =
					call.slots.iter().map(|f| view_of(*f)).collect();
				let same = call.group.as_ref().is_some_and(|(held, _)| {
					held.len() == views.len() && held.iter().zip(&views).all(|(a, b)| a == *b)
				});
				if !same {
					let group = sprites.views_group(gpu, &views);
					call.group = Some((views.into_iter().cloned().collect(), group));
				}
			}
		}
	}

	/// The draw calls of the segments `shown` before the split (`before`) or after it.
	pub(crate) fn calls<'a>(
		&'a self,
		shown: &Shown,
		before: bool,
	) -> Vec<(Range<u32>, &'a wgpu::BindGroup)> {
		shown
			.segments
			.iter()
			.map(|&s| &self.segments[s])
			.filter(|s| (s.items.start < self.split) == before)
			.filter_map(|s| s.placed.as_ref())
			.flat_map(|p| &p.calls)
			.filter(|c| !c.items.is_empty())
			.filter_map(|c| {
				c.group
					.as_ref()
					.map(|(_, g)| (c.items.start as u32..c.items.end as u32, g))
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_sprites::{name_of, registry};

	fn tile(y: f64, details: Vec<SceneDetail>) -> SceneTile {
		SceneTile {
			col: 0,
			row: 0,
			pos: (0.0, y),
			terrain: 2,
			discovered: true,
			brush: "rock".into(),
			flipped: false,
			details,
		}
	}

	fn detail(id: i64, brush: &str, z: i32) -> SceneDetail {
		SceneDetail {
			id,
			brush: brush.into(),
			z,
			..SceneDetail::default()
		}
	}

	fn names(map: &KeptMap) -> Vec<String> {
		map.items
			.iter()
			.map(|q| {
				q.as_ref()
					.map_or("-".to_string(), |q| name_of(q).to_string())
			})
			.collect()
	}

	/// The order is the per-frame build's: terrain, then the details by key, the
	/// negative keys after the split; a hidden detail keeps its place, empty.
	#[test]
	fn the_map_keeps_the_original_order_and_hidden_details_keep_their_place() {
		let mut hidden = detail(3, "mark", 5);
		hidden.hidden = true;
		let tiles = vec![
			tile(0.0, vec![detail(1, "rock", -2), detail(2, "mark", 10)]),
			tile(100.0, vec![hidden, detail(4, "rock", 0)]),
		];
		let mut map = KeptMap::default();
		map.build(&tiles, &registry(), None);
		assert_eq!(names(&map), ["rock", "rock", "mark", "-", "rock", "rock"]);
		assert_eq!(map.split, 5, "before the detail keyed below 0");
		assert_eq!(
			map.segments
				.iter()
				.map(|s| s.items.clone())
				.collect::<Vec<_>>(),
			[0..5, 5..6]
		);

		// Shown again, it's drawn where it was.
		let mut shown = detail(3, "rock", 5);
		shown.pos = (0.0, 100.0);
		map.change(&[shown], &registry());
		assert_eq!(
			names(&map),
			["rock", "rock", "mark", "rock", "rock", "rock"]
		);
		assert_eq!(map.segments[0].patched, [3]);
	}
}
