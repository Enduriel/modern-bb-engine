//! The world map's terrain and details, kept on the GPU from frame to frame.
//!
//! Zoomed out, the world map is ~32,500 sprites, nearly all of them tiles' terrain and
//! details that don't change from one frame to the next. They're built once, in the
//! order they draw, written to a vertex buffer in world units (the camera and the day's
//! light are uniforms), and drawn from it every frame; only what changed is built again:
//!
//! - The map's order changes in a way it can't follow (the map sized, cleared or
//!   loaded): everything is built again
//!   (`squirrel_bb::WorldGrid::map_generation`), as the sprite registry's sheets
//!   changing.
//! - A detail is spawned or removed, or a tile's terrain set
//!   (`WorldGrid::take_map_edits`; every settlement's house lights at dusk and dawn):
//!   its quad goes in at its place in the order, or out, and only its segment is
//!   located and written again.
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
//! The map is cut into segments of consecutive quads of that order, each with room for
//! [`SEGMENT`] in the kept buffer (a build fills [`FILL`], leaving room for details
//! spawned later; a full one is split in two, the second half in new room). The
//! segments are drawn in order wherever their room lies. A segment is drawn if its
//! quads' bounds meet the screen, located in the pages when it's first drawn (a frame
//! then loads as it would have), and drawn in its own calls of up to eight atlases, so
//! skipping one keeps the order of the rest. Its bounds are its quads' own corners, so
//! what it skips is off the screen.

use std::cmp::Reverse;
use std::ops::Range;

use bb_assets::SpriteRegistry;
use squirrel_bb::scene::{MapEdit, SceneDetail, SceneTile};

use crate::fast_hash::FastMap;
use crate::map_quad::MapQuad;
use crate::pages::{FramePages, Located};
use crate::residency::Drawer;
use crate::sprite::{Quad, SLOTS, SpriteRenderer};
use crate::world::{
	AtlasIds, AtlasList, Layer, WorldView, detail_quad, sort_back_to_front, terrain_quad,
};

/// A segment's room in the kept buffer, in quads.
pub(crate) const SEGMENT: usize = 256;

/// The quads a build puts in a segment: the rest of its room waits for details spawned
/// later (a settlement's lights at dusk), so that few segments split.
pub(crate) const FILL: usize = SEGMENT - SEGMENT / 8;

/// What the kept map was built from: the map's generation and the registry's sheets.
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

/// A quad's place in the original's order (`world_quads`' sort): by layer, the higher
/// key first, then in push order, which is the tiles row by row (`col` fastest), and on
/// a tile its terrain and then its details in spawn order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Order {
	layer: Layer,
	key: Reverse<i64>,
	/// The tile's index in the map's order.
	tile: u32,
	/// Its detail's place among the tile's, in spawn order (0 for terrain).
	nth: u32,
}

/// One kept quad.
struct Item {
	/// In its atlas's coordinates, its `texture` indexing `atlases`; none draws nothing
	/// (hidden, or an unknown brush). Kept on the CPU to be located again (see
	/// [`MapQuad`]).
	quad: MapQuad,
	order: Order,
	/// Its detail's id, or for terrain its tile's [`terrain_id`].
	id: i64,
}

/// The id a tile's terrain is kept by (details' ids, entity ids, are never negative).
fn terrain_id(tile: usize) -> i64 {
	i64::MIN + tile as i64
}

/// The map's terrain and details in drawing order.
#[derive(Default)]
pub(crate) struct KeptMap {
	/// `None`: built for one frame only (a scene given whole, see `WorldRenderer::draw`).
	built: Option<Built>,
	atlases: AtlasList,
	/// Every segment, each at the room its index gives (`index * SEGMENT` quads in).
	segments: Vec<Segment>,
	/// The segments in drawing order.
	order: Vec<usize>,
	/// Each quad by its id (see [`Item::id`]): its segment and place in it.
	ids: FastMap<i64, (usize, usize)>,
	/// Each tile's next detail's place among its own ([`Order::nth`]).
	next: Vec<u32>,
	/// Where each frame was found this frame, by atlas and texture coordinates.
	memo: FastMap<(usize, [u32; 8]), Located>,
	/// The pages' generation the segments' bind groups were made at.
	generation: u64,
}

struct Segment {
	/// Drawn before the details layer's own sprites (terrain and details keyed ≥ 0), or
	/// after them (see [`KeptMap::calls`]).
	before: bool,
	items: Vec<Item>,
	/// The quads' extent, `[left, right, bottom, top]` (left > right with none).
	bounds: [f32; 4],
	placed: Option<Placed>,
	/// Quads changed since it was placed (their places in it).
	patched: Vec<usize>,
}

impl Segment {
	fn new(before: bool, items: Vec<Item>) -> Segment {
		let mut segment = Segment {
			before,
			items,
			bounds: EMPTY,
			placed: None,
			patched: Vec::new(),
		};
		segment.measure();
		segment
	}

	/// Its bounds, measured again.
	fn measure(&mut self) {
		self.bounds = EMPTY;
		for quad in self.items.iter().filter_map(|i| i.quad.get()) {
			grow(&mut self.bounds, &quad);
		}
	}

	/// Its quads changed in number or order: it's located and written again whole.
	fn unplace(&mut self) {
		self.placed = None;
		self.patched.clear();
		self.measure();
	}
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
	/// Its quads in the kept buffer.
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

	/// Build it from `tiles` (the whole map in its order, or for `built` `None` what one
	/// frame shows).
	pub(crate) fn build(
		&mut self,
		tiles: &[SceneTile],
		sprites: &SpriteRegistry,
		built: Option<Built>,
	) {
		struct Entry {
			order: Order,
			id: i64,
			quad: Option<(usize, Quad)>,
		}
		let mut ids = AtlasIds::default();
		let mut entries = Vec::new();
		self.next.clear();
		for (t, tile) in tiles.iter().enumerate() {
			let at = |layer, key, nth| Order {
				layer,
				key: Reverse(key),
				tile: t as u32,
				nth,
			};
			if let Some((key, texture, quad)) = terrain_quad(tile, sprites) {
				entries.push(Entry {
					order: at(Layer::Terrain, key, 0),
					id: terrain_id(t),
					quad: Some((ids.id(texture), quad)),
				});
			}
			for (nth, detail) in tile.details.iter().enumerate() {
				let quad = (!detail.hidden)
					.then(|| detail_quad(detail, sprites))
					.flatten();
				entries.push(Entry {
					order: at(Layer::Details, i64::from(detail.z), nth as u32),
					id: detail.id,
					quad: quad.map(|(texture, quad)| (ids.id(texture), quad)),
				});
			}
			self.next.push(tile.details.len() as u32);
		}
		let (mut keys, mut order) = (Vec::new(), Vec::new());
		sort_back_to_front(
			entries.len(),
			|i| (entries[i].order.layer, entries[i].order.key.0),
			&mut keys,
			&mut order,
		);
		let mut atlases = AtlasList::default();
		let mut index = vec![usize::MAX; ids.paths.len()];
		let mut items: Vec<Item> = order
			.iter()
			.map(|&i| {
				let entry = &entries[i as usize];
				Item {
					quad: MapQuad::of(entry.quad.map(|(texture, quad)| {
						let atlas = &mut index[texture];
						if *atlas == usize::MAX {
							*atlas = atlases.index_of(ids.paths[texture]);
						}
						Quad {
							texture: *atlas,
							..quad
						}
					})),
					order: entry.order,
					id: entry.id,
				}
			})
			.collect();
		drop(entries);
		self.atlases = atlases;
		self.segments.clear();
		self.order.clear();
		self.ids.clear();
		// The details keyed below 0 are drawn after the details layer's own sprites.
		let split = items.iter().position(|i| !before(&i.order)).unwrap_or(items.len());
		let after = items.split_off(split);
		for (part, is_before) in [(items, true), (after, false)] {
			let mut part = part.into_iter().peekable();
			while part.peek().is_some() {
				let items: Vec<Item> = part.by_ref().take(FILL).collect();
				self.push_segment(Segment::new(is_before, items));
			}
		}
		self.built = built;
	}

	/// Add `segment` at the end of the drawing order, in new room.
	fn push_segment(&mut self, segment: Segment) -> usize {
		let s = self.segments.len();
		for (at, item) in segment.items.iter().enumerate() {
			self.ids.insert(item.id, (s, at));
		}
		self.segments.push(segment);
		self.order.push(s);
		s
	}

	/// The room the kept buffer needs, in quads.
	pub(crate) fn len(&self) -> usize {
		self.segments.len() * SEGMENT
	}

	/// The kept buffer was made anew: every segment is written again.
	pub(crate) fn unplace(&mut self) {
		for segment in &mut self.segments {
			segment.placed = None;
		}
	}

	/// Follow the details spawned and removed and the tiles' terrain set since it was
	/// built or last edited, in order (see `squirrel_bb::WorldGrid::take_map_edits`).
	pub(crate) fn edit(&mut self, edits: &[MapEdit], sprites: &SpriteRegistry) {
		for edit in edits {
			match edit {
				MapEdit::Spawned { tile, detail } => self.spawned(*tile, detail, sprites),
				MapEdit::Removed(id) => self.remove(*id),
				MapEdit::Terrain { tile, terrain } => {
					// Its key follows its brush's depth bias: out, and in at its place.
					self.remove(terrain_id(*tile));
					if let Some((key, texture, quad)) = terrain_quad(terrain, sprites) {
						let quad = Quad {
							texture: self.atlases.index_of(texture),
							..quad
						};
						self.insert(Item {
							quad: MapQuad::of(Some(quad)),
							order: Order {
								layer: Layer::Terrain,
								key: Reverse(key),
								tile: *tile as u32,
								nth: 0,
							},
							id: terrain_id(*tile),
						});
					}
				}
			}
		}
	}

	/// `detail`, spawned on tile `tile` after its others, goes in at its place.
	fn spawned(&mut self, tile: usize, detail: &SceneDetail, sprites: &SpriteRegistry) {
		if self.next.len() <= tile {
			self.next.resize(tile + 1, 0);
		}
		let nth = self.next[tile];
		self.next[tile] += 1;
		let item = Item {
			quad: MapQuad::of(self.detail_quad(detail, sprites)),
			order: Order {
				layer: Layer::Details,
				key: Reverse(i64::from(detail.z)),
				tile: tile as u32,
				nth,
			},
			id: detail.id,
		};
		self.insert(item);
	}

	/// `item` goes in at its place in the order.
	fn insert(&mut self, item: Item) {
		let is_before = before(&item.order);
		// The first segment of its part whose last quad comes after it, or the part's
		// last (a part without any gets one, at the split).
		let (mut found, mut last) = (None, None);
		for (at, &s) in self.order.iter().enumerate() {
			let segment = &self.segments[s];
			if segment.before != is_before {
				continue;
			}
			last = Some(at);
			if segment.items.last().is_some_and(|i| i.order > item.order) {
				found = Some(at);
				break;
			}
		}
		let at = match found.or(last) {
			Some(at) => at,
			None => {
				let s = self.segments.len();
				self.segments.push(Segment::new(is_before, Vec::new()));
				let at = if is_before {
					self.order
						.iter()
						.position(|&s| !self.segments[s].before)
						.unwrap_or(self.order.len())
				} else {
					self.order.len()
				};
				self.order.insert(at, s);
				at
			}
		};
		let mut s = self.order[at];
		let mut place = self.segments[s].items.partition_point(|i| i.order < item.order);
		if self.segments[s].items.len() == SEGMENT {
			// Full: its second half goes to new room, after it.
			let half = SEGMENT / 2;
			let moved = self.segments[s].items.split_off(half);
			let is_before = self.segments[s].before;
			self.segments[s].unplace();
			let new = self.segments.len();
			for (i, moved) in moved.iter().enumerate() {
				self.ids.insert(moved.id, (new, i));
			}
			self.segments.push(Segment::new(is_before, moved));
			self.order.insert(at + 1, new);
			if place > half {
				s = new;
				place -= half;
			}
		}
		let segment = &mut self.segments[s];
		segment.items.insert(place, item);
		segment.unplace();
		for (i, item) in segment.items.iter().enumerate().skip(place) {
			self.ids.insert(item.id, (s, i));
		}
	}

	/// The quad with id `id` goes out of the order (one never kept is passed over).
	fn remove(&mut self, id: i64) {
		let Some((s, place)) = self.ids.remove(&id) else {
			return;
		};
		let segment = &mut self.segments[s];
		segment.items.remove(place);
		segment.unplace();
		for (i, item) in segment.items.iter().enumerate().skip(place) {
			self.ids.insert(item.id, (s, i));
		}
	}

	/// `detail`'s quad as kept: `None` if hidden or its brush is unknown.
	fn detail_quad(&mut self, detail: &SceneDetail, sprites: &SpriteRegistry) -> Option<Quad> {
		let quad = (!detail.hidden)
			.then(|| detail_quad(detail, sprites))
			.flatten();
		quad.map(|(texture, quad)| Quad {
			texture: self.atlases.index_of(texture),
			..quad
		})
	}

	/// Take the details whose look changed: their quads, built again.
	pub(crate) fn change(&mut self, changes: &[SceneDetail], sprites: &SpriteRegistry) {
		for detail in changes {
			let Some(&(s, at)) = self.ids.get(&detail.id) else {
				continue;
			};
			let quad = self.detail_quad(detail, sprites);
			let kept = MapQuad::of(quad);
			let segment = &mut self.segments[s];
			if kept == segment.items[at].quad {
				continue;
			}
			segment.items[at].quad = kept;
			if let Some(quad) = &quad {
				grow(&mut segment.bounds, quad);
			}
			segment.patched.push(at);
		}
	}

	/// The segments `view` shows, in drawing order.
	pub(crate) fn shown(&self, view: &WorldView) -> Shown {
		let [left, right, bottom, top] = view.bounds(0.0);
		Shown {
			segments: self
				.order
				.iter()
				.copied()
				.filter(|&s| {
					let [l, r, b, t] = self.segments[s].bounds;
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

	/// Where quad `at` of segment `s` is drawn from.
	fn locate(&mut self, s: usize, at: usize, cx: &mut Placing) -> At {
		let Some(quad) = self.segments[s].items[at].quad.get() else {
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
		let base = s * SEGMENT;
		let mut writes = Vec::new();
		for &i in &patched {
			let at = self.locate(s, i, cx);
			let placed = self.segments[s].placed.as_mut().expect("placed");
			let call = placed
				.calls
				.iter_mut()
				.find(|c| c.items.contains(&(base + i)))
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
			placed.drawn[i] = drawn.is_some();
			writes.push((base + i, drawn));
		}
		for (i, drawn) in writes {
			cx.sprites.write_kept(i, [drawn]);
		}
		true
	}

	/// Locate segment `s` whole, batch it in calls and write it to its room.
	fn place_segment(&mut self, s: usize, generation: u64, cx: &mut Placing) {
		let (base, len) = (s * SEGMENT, self.segments[s].items.len());
		self.segments[s].patched.clear();
		let mut placed = Placed {
			generation,
			calls: vec![KeptCall {
				items: base..base,
				slots: Vec::new(),
				group: None,
			}],
			pages: Vec::new(),
			whole: Vec::new(),
			drawn: Vec::with_capacity(len),
			waiting: 0,
		};
		let mut writes = Vec::with_capacity(len);
		for i in 0..len {
			let drawn = match self.locate(s, i, cx) {
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
									items: base + i..base + i,
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
			placed.calls.last_mut().expect("a call").items.end = base + i + 1;
			writes.push(drawn);
		}
		cx.sprites.write_kept(base, writes);
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
	) -> impl Iterator<Item = Quad> + 'a {
		self.shown_quads(shown).filter(move |q| q.texture == atlas)
	}

	/// The quads of the segments `shown`, in their atlases' coordinates (usage tracking).
	pub(crate) fn shown_quads<'a>(
		&'a self,
		shown: &'a Shown,
	) -> impl Iterator<Item = Quad> + 'a {
		shown
			.segments
			.iter()
			.flat_map(|&s| self.segments[s].items.iter().filter_map(|i| i.quad.get()))
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

	/// The draw calls of the segments `shown` drawn before the details layer's own
	/// sprites (`before`: terrain and details keyed ≥ 0) or after them.
	pub(crate) fn calls<'a>(
		&'a self,
		shown: &Shown,
		before: bool,
	) -> Vec<(Range<u32>, &'a wgpu::BindGroup)> {
		shown
			.segments
			.iter()
			.map(|&s| &self.segments[s])
			.filter(|s| s.before == before)
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

/// Whether a quad of `order` is drawn before the details layer's own sprites: the
/// terrain and the details keyed ≥ 0.
fn before(order: &Order) -> bool {
	order.layer != Layer::Details || order.key.0 >= 0
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

	/// Each kept quad in drawing order: what it shows (`-` for nothing).
	fn names(map: &KeptMap) -> Vec<String> {
		items(map)
			.iter()
			.map(|(q, _, _)| q.as_ref().map_or("-".to_string(), |q| name_of(q).to_string()))
			.collect()
	}

	/// Each kept quad in drawing order, its detail's id, and whether it's drawn before
	/// the details layer's own sprites.
	fn items(map: &KeptMap) -> Vec<(Option<Quad>, i64, bool)> {
		map.order
			.iter()
			.flat_map(|&s| {
				let segment = &map.segments[s];
				segment.items.iter().map(|i| (i.quad.get(), i.id, segment.before))
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
		assert_eq!(
			map.order
				.iter()
				.map(|&s| (map.segments[s].items.len(), map.segments[s].before))
				.collect::<Vec<_>>(),
			[(5, true), (1, false)],
			"split before the detail keyed below 0"
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

	/// Details spawned and removed and terrain set, one by one, give the map a build of
	/// the tiles as they end up gives, quad for quad: keys ≥ 0 and below, ties between
	/// tiles and on one tile, hidden ones, terrain changed or gone, a part with no
	/// segment yet, and segments filling and splitting.
	#[test]
	fn edits_keep_the_order_a_build_gives() {
		let registry = registry();
		let brushes = ["rock", "mark", "step", "drop"];
		// A small generator (xorshift), so the run is the same every time.
		let mut state = 0x2545_f491_4f6c_dd1du64;
		let mut next = move |n: u64| {
			state ^= state << 13;
			state ^= state >> 7;
			state ^= state << 17;
			state % n
		};
		let mut id = 0;
		let mut new_detail = |next: &mut dyn FnMut(u64) -> u64, tile: usize| {
			id += 1;
			let mut d = detail(id, brushes[next(4) as usize], next(9) as i32 - 3);
			d.hidden = next(10) == 0;
			d.pos = (tile as f32 * 3.0, next(50) as f32);
			d
		};
		// Only details keyed ≥ 0 at first: the part after the split has no segment.
		let mut tiles: Vec<SceneTile> = (0..40)
			.map(|t| {
				let mut tile = tile(t as f64, Vec::new());
				for _ in 0..next(12) {
					let mut d = new_detail(&mut next, t);
					d.z = d.z.abs();
					tile.details.push(d);
				}
				tile
			})
			.collect();
		let mut map = KeptMap::default();
		map.build(&tiles, &registry, None);
		assert!(map.segments.iter().all(|s| s.before));
		let segments = map.segments.len();
		for round in 0..12 {
			let mut edits = Vec::new();
			for _ in 0..60 {
				let t = next(tiles.len() as u64) as usize;
				if next(8) == 0 {
					// Its terrain set: another brush, or none.
					tiles[t].brush = ["rock", "tile", ""][next(3) as usize].into();
					tiles[t].flipped = next(2) == 0;
					let terrain = SceneTile { details: Vec::new(), ..tiles[t].clone() };
					edits.push(MapEdit::Terrain { tile: t, terrain });
				} else if next(3) == 0 && !tiles[t].details.is_empty() {
					let at = next(tiles[t].details.len() as u64) as usize;
					let gone = tiles[t].details.remove(at);
					edits.push(MapEdit::Removed(gone.id));
				} else {
					// Crowd a few tiles, so that segments fill and split.
					let t = if round % 2 == 0 { t % 3 } else { t };
					let d = new_detail(&mut next, t);
					tiles[t].details.push(d.clone());
					edits.push(MapEdit::Spawned { tile: t, detail: d });
				}
			}
			map.edit(&edits, &registry);
			let mut built = KeptMap::default();
			built.build(&tiles, &registry, None);
			assert_eq!(items(&map), items(&built), "round {round}");
			for (&id, &(s, at)) in &map.ids {
				assert_eq!(map.segments[s].items[at].id, id, "where quad {id} is kept");
			}
			assert!(map.segments.iter().all(|s| s.items.len() <= SEGMENT));
		}
		assert!(map.segments.len() > segments, "segments split, and a part began");
		assert!(map.segments.iter().any(|s| !s.before));
	}
}
