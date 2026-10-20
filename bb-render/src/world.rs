//! Drawing the world map from a [`WorldScene`]: tiles, their details and the
//! entities' sprites, as quads for the sprite program.
//!
//! Checked against screenshots of the original (at zoom 1 the render
//! lines up at a (0, 0) offset):
//! - world units are pixels at zoom 1, y up (tile rows grow north);
//! - a frame's rect is y up from the sprite's anchor, and its `top` is the edge at the
//!   atlas's first rows, which is the picture's bottom: bbrusher packs pictures
//!   upside down, the way GL reads rows bottom-up;
//! - brush and sprite offsets are y up; a mirror is about the anchor.
//!
//! Draw order: tile terrain, then the details layer, then the entities layer (the
//! original keeps one quadtree per scene layer). Details sort back to front on the key
//! their spawn gave them (`spawnDetail`, `0x4fb707`): `int(y) + depth_bias + z level`
//! minus the spawn tile's axial-r parity, which decides between neighbouring tiles'
//! pieces (the mountains' overlaps show it); ties keep spawn order, and entities sort
//! north to south, which is not yet checked.
//! Region names are text nodes in the details layer whose key stays 0 (`0x4f0350`),
//! so they draw over the details.

use std::collections::HashMap;

use bb_assets::{BrushFrame, SpriteRegistry};
use squirrel_bb::scene::{SceneDetail, SceneTile, WorldScene};

use crate::fast_hash::FastMap;
use crate::labels::LabelFont;
use crate::sprite::{Quad, SceneUniforms};

/// The bitmap fonts world text draws in (`None` where the data lacks one).
#[derive(Clone, Copy, Default)]
pub struct WorldFonts<'a> {
	/// Entity labels (`gfx/fonts/cinzel_bold_20`).
	pub label: Option<LabelFont<'a>>,
	/// Region names (`gfx/fonts/cinzel_bold_100`, `0x47dcbc`).
	pub region: Option<LabelFont<'a>>,
	/// The outline fonts, for text the bitmap fonts can't draw (`None`: such text isn't
	/// drawn).
	pub shaper: Option<&'a std::cell::RefCell<crate::fonts::shaper::TextShaper>>,
}

/// How far a cloud's centre may be off screen and still show (cloud pictures are
/// large and scaled up to 1.25, their shadows 1.25 more and 500 units lower).
pub(crate) const CLOUD_REACH: f32 = 2000.0;

/// A cloud's shadow: 1.25 of its scale, a fifth of its alpha, 500 units times its
/// scale below it (`0x4e3ed0`).
pub(crate) const CLOUD_SHADOW_SCALE: f32 = 1.25;
pub(crate) const CLOUD_SHADOW_ALPHA: f32 = 0.2;
pub(crate) const CLOUD_SHADOW_DROP: f32 = -500.0;

/// How far past its anchor a tile's or an entity's sprites can reach: well past the hex
/// (400 x 200 around a 180 x 120 cell).
pub(crate) const SPRITE_REACH: f32 = 400.0;

/// Region names: white at alpha `0x66`, at 0.4 of the script's scale (`0x4f0350`).
pub(crate) const REGION_TEXT_COLOR: [u8; 4] = [255, 255, 255, 0x66];
pub(crate) const REGION_TEXT_SCALE: f32 = 0.4;

/// What part of the world is shown.
#[derive(Clone, Copy, Debug)]
pub struct WorldView {
	/// The world point at the centre of the screen.
	pub center: [f32; 2],
	/// Screen pixels per world unit.
	pub zoom: f32,
	pub width: u32,
	pub height: u32,
}

impl WorldView {
	/// The sprite program's uniforms for this view (neutral ambient light).
	pub fn uniforms(&self) -> SceneUniforms {
		SceneUniforms::centered(
			self.center,
			self.width as f32 / self.zoom,
			self.height as f32 / self.zoom,
		)
	}

	/// Whether a tile or entity anchored at world point `pos` can show in this view: what
	/// [`world_quads`] draws of them, so a scene can be cut down to it beforehand.
	pub fn shows(&self, (x, y): (f64, f64)) -> bool {
		let [left, right, bottom, top] = self.bounds(SPRITE_REACH);
		let (x, y) = (x as f32, y as f32);
		x >= left && x <= right && y >= bottom && y <= top
	}

	/// The world rectangle on screen, `[left, right, bottom, top]`, grown by `margin`.
	pub(crate) fn bounds(&self, margin: f32) -> [f32; 4] {
		let half_w = self.width as f32 / self.zoom / 2.0 + margin;
		let half_h = self.height as f32 / self.zoom / 2.0 + margin;
		[
			self.center[0] - half_w,
			self.center[0] + half_w,
			self.center[1] - half_h,
			self.center[1] + half_h,
		]
	}
}

/// One sprite to draw, before sorting. Its quad's `texture` is its atlas's id in the
/// frame's [`AtlasIds`].
struct Drawable {
	/// Scene layer, drawn in turn: tile terrain, details, entities.
	layer: Layer,
	/// Within a layer, a higher key draws first (further back); ties in push order.
	key: i64,
	quad: Quad,
}

/// The atlases sprites name while a frame's quads are built, by an id each: an atlas's
/// path is mostly the same string (its sheet's), so it's found by where the string is,
/// without hashing its text for every sprite.
#[derive(Default)]
pub(crate) struct AtlasIds<'a> {
	pub(crate) paths: Vec<&'a str>,
	by_address: FastMap<(usize, usize), usize>,
}

impl<'a> AtlasIds<'a> {
	pub(crate) fn id(&mut self, path: &'a str) -> usize {
		let paths = &mut self.paths;
		*self.by_address.entry((path.as_ptr() as usize, path.len())).or_insert_with(|| {
			paths.push(path);
			paths.len() - 1
		})
	}
}

/// A frame's quads in the making, kept from frame to frame so that drawing tens of
/// thousands of sprites (the map zoomed out) allocates nothing per frame (see
/// [`world_quads_with`]).
#[derive(Default)]
pub struct QuadScratch {
	drawables: Vec<Drawable>,
	order: Vec<u32>,
	keys: Vec<u64>,
	quads: Vec<Quad>,
}

impl QuadScratch {
	/// Take a frame's quads back once drawn, for the next frame to fill.
	pub fn recycle(&mut self, quads: WorldQuads) {
		self.quads = quads.quads;
	}
}

/// The atlases a frame uses, by texture path, in first-use order.
#[derive(Default)]
pub struct AtlasList {
	pub paths: Vec<String>,
	index: HashMap<String, usize>,
}

impl AtlasList {
	pub(crate) fn index_of(&mut self, path: &str) -> usize {
		if let Some(&i) = self.index.get(path) {
			return i;
		}
		self.paths.push(path.to_string());
		self.index.insert(path.to_string(), self.paths.len() - 1);
		self.paths.len() - 1
	}
}

/// The world's scene layers, in drawing order. The original keeps one quadtree per
/// layer (details, entities, effects) and draws them in turn; tile terrain is drawn
/// before all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Layer {
	Terrain,
	Details,
	Entities,
	/// Clouds and their shadows (the original's effects layer).
	Effects,
	/// Entity labels, above everything.
	Labels,
}

/// A bird's alpha, and its shadow's alpha, scale and offset down (the bird's two
/// sprites, `0x4e3ad0`). The shadow's colour comes from a global set at startup
/// (`0x68f0a8`) that hasn't been read; it's black here, as the clouds' shadows are.
pub(crate) const BIRD_ALPHA: u8 = 0xa8;
pub(crate) const BIRD_SHADOW_ALPHA: u8 = 0x10;
pub(crate) const BIRD_SHADOW_SCALE: f32 = 1.25;
pub(crate) const BIRD_SHADOW_DROP: f32 = 150.0;

/// A frame's quads in drawing order, and the atlases they index.
pub struct WorldQuads {
	pub quads: Vec<Quad>,
	pub atlases: AtlasList,
	/// Where the labels start in `quads` (they draw over the fog of war).
	pub labels_from: usize,
	/// Where the details layer ends in `quads` (the map's kept terrain and details draw
	/// around what of it is built each frame, see `crate::kept_map`).
	pub details_end: usize,
}

/// The quads that draw `scene` in `view`.
pub fn world_quads<'a>(
	scene: &'a WorldScene,
	sprites: &'a SpriteRegistry,
	view: &WorldView,
	fonts: WorldFonts<'a>,
) -> WorldQuads {
	world_quads_with(scene, sprites, view, fonts, &mut QuadScratch::default())
}

/// [`world_quads`], building in `scratch`'s memory (give the quads back with
/// [`QuadScratch::recycle`]).
pub fn world_quads_with<'a>(
	scene: &'a WorldScene,
	sprites: &'a SpriteRegistry,
	view: &WorldView,
	fonts: WorldFonts<'a>,
	scratch: &mut QuadScratch,
) -> WorldQuads {
	quads_of(scene, sprites, view, fonts, scratch, true)
}

/// [`world_quads_with`] without the tiles' terrain and details: what changes from frame
/// to frame around the map kept on the GPU (see `crate::kept_map`).
pub(crate) fn world_quads_around_map<'a>(
	scene: &'a WorldScene,
	sprites: &'a SpriteRegistry,
	view: &WorldView,
	fonts: WorldFonts<'a>,
	scratch: &mut QuadScratch,
) -> WorldQuads {
	quads_of(scene, sprites, view, fonts, scratch, false)
}

fn quads_of<'a>(
	scene: &'a WorldScene,
	sprites: &'a SpriteRegistry,
	view: &WorldView,
	fonts: WorldFonts<'a>,
	scratch: &mut QuadScratch,
	tiles: bool,
) -> WorldQuads {
	let on_screen = |pos: (f64, f64)| view.shows(pos);
	let QuadScratch { drawables, order, keys, quads } = scratch;
	drawables.clear();
	let mut ids = AtlasIds::default();
	for tile in scene.tiles.iter().filter(|t| tiles && on_screen(t.pos)) {
		if let Some((key, texture, quad)) = terrain_quad(tile, sprites) {
			drawables.push(Drawable {
				layer: Layer::Terrain,
				key,
				quad: Quad { texture: ids.id(texture), ..quad },
			});
		}
		for detail in tile.details.iter().filter(|d| !d.hidden) {
			if let Some((texture, quad)) = detail_quad(detail, sprites) {
				drawables.push(Drawable {
					layer: Layer::Details,
					key: i64::from(detail.z),
					quad: Quad { texture: ids.id(texture), ..quad },
				});
			}
		}
	}
	// The moving layer, item by item (see `crate::world_items`).
	let mut item_quads = Vec::new();
	for item in crate::world_items::items(scene, view, fonts).filter(|item| item.shows(view)) {
		item.quads(sprites, fonts, &mut item_quads);
		for q in item_quads.drain(..) {
			drawables.push(Drawable {
				layer: q.layer,
				key: q.key,
				quad: Quad {
					texture: ids.id(q.texture),
					..q.quad
				},
			});
		}
	}

	// Back to front: by layer, then the higher key first, ties in push order. The order
	// is unique, so sorting compact keys (and gathering after) gives the same order as
	// sorting the drawables themselves, at a fraction of the moves.
	sort_back_to_front(drawables.len(), |i| (drawables[i].layer, drawables[i].key), keys, order);
	let labels_from = order
		.iter()
		.position(|&i| drawables[i as usize].layer == Layer::Labels)
		.unwrap_or(order.len());
	let details_end = order
		.iter()
		.position(|&i| drawables[i as usize].layer > Layer::Details)
		.unwrap_or(order.len());
	// The atlases in first-use order.
	let mut atlases = AtlasList::default();
	let mut index = vec![usize::MAX; ids.paths.len()];
	let mut quads = std::mem::take(quads);
	quads.clear();
	quads.extend(order.iter().map(|&i| {
		let d = &drawables[i as usize];
		let at = &mut index[d.quad.texture];
		if *at == usize::MAX {
			*at = atlases.index_of(ids.paths[d.quad.texture]);
		}
		Quad { texture: *at, ..d.quad }
	}));
	WorldQuads {
		quads,
		atlases,
		labels_from,
		details_end,
	}
}

/// Sprite `brush`'s quad showing `frame` of it (its first if it has no such frame) at
/// `anchor` plus `offset` (both scaled with it), drawn as `look` says: its atlas, its
/// depth bias and its quad (texture 0). `None` for an unknown brush or one without frames.
pub(crate) fn sprite_quad<'a>(
	sprites: &'a SpriteRegistry,
	brush: &str,
	frame: u32,
	anchor: [f32; 2],
	offset: [f32; 2],
	look: Look,
) -> Option<(&'a str, i8, Quad)> {
	let (sheet, sprite) = sprites.get(brush)?;
	// The frame its animation shows (see `squirrel_bb::native::sprite_anim`).
	let frame = sprite.frames.get(frame as usize).or(sprite.frames.first())?;
	// Offsets scale with the sprite (so an unscaled sprite keeps its screen offset too).
	let anchor = [
		anchor[0] + (offset[0] + f32::from(sprite.offset_x)) * look.scale,
		anchor[1] + (offset[1] + f32::from(sprite.offset_y)) * look.scale,
	];
	Some((&sheet.sheet.texture, sprite.depth_bias, frame_quad(frame, anchor, look)))
}

/// A tile's terrain: its sort key in the terrain layer (its row's y plus the brush's
/// depth bias), its atlas and its quad. `None` without a brush.
pub(crate) fn terrain_quad<'a>(tile: &SceneTile, sprites: &'a SpriteRegistry) -> Option<(i64, &'a str, Quad)> {
	if tile.brush.is_empty() {
		return None;
	}
	let anchor = [tile.pos.0 as f32, tile.pos.1 as f32];
	let (texture, depth_bias, mut quad) = sprite_quad(sprites, &tile.brush, 0, anchor, [0.0; 2], Look::PLAIN)?;
	// A flipped tile swaps its texture's left and right, in place (`0x573ab3`).
	if tile.flipped {
		let uv = &mut quad.uv;
		(uv[0][0], uv[1][0], uv[2][0], uv[3][0]) = (uv[1][0], uv[0][0], uv[3][0], uv[2][0]);
	}
	Some((anchor[1] as i64 + i64::from(depth_bias), texture, quad))
}

/// A detail's atlas and quad, at its place (its tile's centre plus the spawn's offset).
/// Its sort key in the details layer is its own `z`, as the spawn made it (`0x4fb707`):
/// int(y) + depth bias + z level − the spawn tile's axial-r parity. `None` for an unknown
/// brush (a hidden detail is drawn as any other: the caller leaves it out).
pub(crate) fn detail_quad<'a>(detail: &SceneDetail, sprites: &'a SpriteRegistry) -> Option<(&'a str, Quad)> {
	let look = Look {
		scale: detail.scale as f32,
		flipped: detail.flipped,
		alpha: detail.alpha as f32,
		rotation: detail.rotation as f32,
		tint: [(detail.tint >> 16) as u8, (detail.tint >> 8) as u8, detail.tint as u8],
	};
	let (texture, _, quad) = sprite_quad(sprites, &detail.brush, detail.frame, [detail.pos.0, detail.pos.1], [0.0; 2], look)?;
	// Its `Saturation` (the steppe's brush is 0.8), and `IgnoreAmbientColor` (a house's lit
	// windows at night, see [`unlit`]).
	Some((
		texture,
		Quad {
			saturation: detail.saturation as f32,
			flags: unlit(detail.ignore_ambient),
			..quad
		},
	))
}

/// Put in `order` the drawables' indices back to front: by layer, then the higher key
/// first, ties in push order (`order` is a drawable's index). That order is unique, so
/// sorting compact keys and gathering after gives the same as sorting the drawables
/// themselves, at a fraction of the moves; the keys pack into one `u64` when they fit
/// (they always do on vanilla's maps), which sorts fastest.
pub(crate) fn sort_back_to_front(count: usize, layer_key: impl Fn(usize) -> (Layer, i64), keys: &mut Vec<u64>, order: &mut Vec<u32>) {
	order.clear();
	keys.clear();
	// Layer: 3 bits; key: 32, reversed; index: 29.
	const INDEX_BITS: u32 = 29;
	let fits = count < 1 << INDEX_BITS && (0..count).all(|i| i32::try_from(layer_key(i).1).is_ok());
	if fits {
		keys.extend((0..count).map(|i| {
			let (layer, key) = layer_key(i);
			let reversed = u64::from(!((key as i32 as u32) ^ 0x8000_0000));
			(layer as u64) << 61 | reversed << INDEX_BITS | i as u64
		}));
		keys.sort_unstable();
		order.extend(keys.iter().map(|k| (k & ((1 << INDEX_BITS) - 1)) as u32));
	} else {
		order.extend(0..count as u32);
		order.sort_unstable_by_key(|&i| {
			let (layer, key) = layer_key(i as usize);
			(layer, std::cmp::Reverse(key), i)
		});
	}
}

/// How a sprite is drawn, besides where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
	pub scale: f32,
	/// Mirrored left to right, about the anchor.
	pub flipped: bool,
	/// 0 to 255.
	pub alpha: f32,
	/// Degrees, counter-clockwise about the anchor.
	pub rotation: f32,
	/// The sprite's colour, RGB (white leaves the picture as it is).
	pub tint: [u8; 3],
}

impl Look {
	/// Unscaled, unmirrored, opaque, upright.
	pub const PLAIN: Look = Look {
		scale: 1.0,
		flipped: false,
		alpha: 255.0,
		rotation: 0.0,
		tint: [255; 3],
	};
}

/// The quad flags for a sprite's `IgnoreAmbientColor` (sprite flag bit 27): the original
/// writes vertex flag 1 for it (`0x52cd82`), and the sprite program leaves a quad with
/// flags out of the ambient colour (the night's blue, the dawn's orange).
pub fn unlit(ignore_ambient: bool) -> u32 {
	u32::from(ignore_ambient)
}

/// A frame drawn at `anchor` (world units, y up).
pub fn frame_quad(frame: &BrushFrame, anchor: [f32; 2], look: Look) -> Quad {
	let [mut left, mut right, top, bottom] = frame.rect;
	let [mut u0, mut u1, v0, v1] = frame.uv;
	if look.flipped {
		// Mirrored about the anchor: the rect flips with the picture.
		(left, right) = (-right, -left);
		std::mem::swap(&mut u0, &mut u1);
	}
	let scale = look.scale;
	let (x0, x1) = (left * scale, right * scale);
	// The rect is y up from the anchor, and its `top` is the edge at the atlas's first
	// rows (`v0`): with the picture packed upside down, that is the picture's bottom.
	let (y_top, y_bottom) = (bottom * scale, top * scale);
	// Upright, the turn is the identity (`x·1 − y·0` is `x`): most sprites skip the trig.
	let (sin, cos) = if look.rotation == 0.0 { (0.0, 1.0) } else { look.rotation.to_radians().sin_cos() };
	let place = |x: f32, y: f32| [anchor[0] + x * cos - y * sin, anchor[1] + x * sin + y * cos];
	Quad {
		texture: 0,
		corners: [
			place(x0, y_top),
			place(x1, y_top),
			place(x1, y_bottom),
			place(x0, y_bottom),
		],
		uv: [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
		color: [
			look.tint[0],
			look.tint[1],
			look.tint[2],
			look.alpha.clamp(0.0, 255.0) as u8,
		],
		corner_colors: None,
		overlay: [0; 4],
		factor: 0.0,
		saturation: 1.0,
		flags: 0,
		level: 0,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_sprites::{frame_of, registry};
	use squirrel_bb::scene::{SceneBird, WorldScene};

	/// The middle of a quad, and its width.
	fn middle_and_width(q: &Quad) -> ([f32; 2], f32) {
		let xs = q.corners.map(|c| c[0]);
		let ys = q.corners.map(|c| c[1]);
		let (lo_x, hi_x) = (xs.iter().copied().fold(f32::MAX, f32::min), xs.iter().copied().fold(f32::MIN, f32::max));
		let (lo_y, hi_y) = (ys.iter().copied().fold(f32::MAX, f32::min), ys.iter().copied().fold(f32::MIN, f32::max));
		([(lo_x + hi_x) / 2.0, (lo_y + hi_y) / 2.0], hi_x - lo_x)
	}

	#[test]
	fn a_bird_draws_over_its_shadow_at_its_frame() {
		let scene = WorldScene {
			birds: vec![SceneBird {
				brush: "anim",
				pos: (0.0, 0.0),
				velocity: 50.0,
				key: -4500,
				frame: 2,
			}],
			..WorldScene::default()
		};
		let view = WorldView {
			center: [0.0, 0.0],
			zoom: 1.0,
			width: 800,
			height: 600,
		};
		let quads = world_quads(&scene, &registry(), &view, WorldFonts::default()).quads;
		assert_eq!(quads.len(), 2, "the shadow, then the bird");
		let (shadow, bird) = (&quads[0], &quads[1]);
		assert_eq!((frame_of(shadow), frame_of(bird)), (2, 2));
		assert_eq!(shadow.color, [0, 0, 0, BIRD_SHADOW_ALPHA]);
		assert_eq!(bird.color, [255, 255, 255, BIRD_ALPHA]);
		let ((s_at, s_width), (b_at, b_width)) = (middle_and_width(shadow), middle_and_width(bird));
		assert_eq!(s_width, b_width * BIRD_SHADOW_SCALE);
		assert_eq!(s_at[1], b_at[1] - BIRD_SHADOW_DROP * BIRD_SHADOW_SCALE, "below it, the offset scaled");
	}

	#[test]
	fn details_sort_on_their_spawns_key_at_their_own_place() {
		use squirrel_bb::scene::{SceneDetail, SceneTile};
		let detail = |brush: &str, z: i32, pos: (f32, f32)| {
			let mut detail = SceneDetail::default();
			detail.brush = brush.into();
			detail.z = z;
			detail.pos = pos;
			detail
		};
		// The spawn's key (with the row parity) decides, not the tile's y: the second,
		// spawned later and further forward by y alone, has the larger key and draws
		// first.
		let scene = WorldScene {
			tiles: vec![SceneTile {
				col: 0,
				row: 0,
				pos: (0.0, 0.0),
				terrain: 2,
				discovered: true,
				brush: "".into(),
				flipped: false,
				details: vec![detail("rock", 29, (0.0, 0.0)), detail("mark", 30, (10.0, -5.0))],
			}],
			..WorldScene::default()
		};
		let view = WorldView {
			center: [0.0, 0.0],
			zoom: 1.0,
			width: 800,
			height: 600,
		};
		let quads = world_quads(&scene, &registry(), &view, WorldFonts::default()).quads;
		let names: Vec<_> = quads.iter().map(crate::test_sprites::name_of).collect();
		assert_eq!(names, ["mark", "rock"]);
		// At its own place: the same detail at the tile's centre sits (10, -5) away.
		let mut moved = scene.clone();
		moved.tiles[0].details[1].pos = (0.0, 0.0);
		let at_centre = world_quads(&moved, &registry(), &view, WorldFonts::default()).quads;
		let mark = |qs: &[Quad]| qs.iter().find(|q| crate::test_sprites::name_of(q) == "mark").unwrap().corners[0];
		let (a, b) = (mark(&quads), mark(&at_centre));
		assert_eq!([a[0] - b[0], a[1] - b[1]], [10.0, -5.0], "at its own place");
	}

	#[test]
	fn a_detail_that_ignores_the_ambient_is_unlit() {
		use squirrel_bb::scene::{SceneDetail, SceneTile};
		let detail = |brush: &str, ignore_ambient| {
			let mut detail = SceneDetail::default();
			detail.brush = brush.into();
			detail.ignore_ambient = ignore_ambient;
			detail
		};
		let scene = WorldScene {
			tiles: vec![SceneTile {
				col: 0,
				row: 0,
				pos: (0.0, 0.0),
				terrain: 2,
				discovered: true,
				brush: "".into(),
				flipped: false,
				details: vec![detail("rock", false), detail("mark", true)],
			}],
			..WorldScene::default()
		};
		let view = WorldView {
			center: [0.0, 0.0],
			zoom: 1.0,
			width: 800,
			height: 600,
		};
		let quads = world_quads(&scene, &registry(), &view, WorldFonts::default()).quads;
		let flags: Vec<_> = quads.iter().map(|q| (crate::test_sprites::name_of(q), q.flags)).collect();
		assert_eq!(flags, [("rock", 0), ("mark", 1)], "the house's lit windows keep their colour at night");
	}

	fn frame(rect: [f32; 4]) -> BrushFrame {
		BrushFrame {
			name: String::new(),
			uv: [0.1, 0.2, 0.3, 0.4],
			rect,
		}
	}

	#[test]
	fn a_frame_sits_on_its_anchor_with_the_atlas_flipped() {
		let quad = frame_quad(
			&frame([-10.0, 10.0, -20.0, 5.0]),
			[100.0, 50.0],
			Look::PLAIN,
		);
		// The rect is y up: the picture spans 20 below the anchor to 5 above, and its
		// top edge takes the atlas's last rows (`v1`).
		assert_eq!(
			quad.corners,
			[[90.0, 55.0], [110.0, 55.0], [110.0, 30.0], [90.0, 30.0]]
		);
		assert_eq!(quad.uv[0], [0.1, 0.4]);
		assert_eq!(quad.uv[2], [0.2, 0.3]);
	}

	#[test]
	fn mirroring_flips_the_rect_about_the_anchor() {
		let look = Look {
			scale: 2.0,
			flipped: true,
			alpha: 128.0,
			rotation: 0.0,
			tint: [255; 3],
		};
		let quad = frame_quad(&frame([-10.0, 4.0, -20.0, 5.0]), [0.0, 0.0], look);
		assert_eq!(quad.corners[0], [-8.0, 10.0]);
		assert_eq!(quad.corners[1], [20.0, 10.0]);
		assert_eq!(quad.uv[0], [0.2, 0.4]);
		assert_eq!(quad.color[3], 128);
	}

	#[test]
	fn rotation_turns_the_quad_about_the_anchor() {
		let look = Look {
			rotation: 90.0,
			..Look::PLAIN
		};
		let quad = frame_quad(&frame([-1.0, 1.0, -1.0, 1.0]), [10.0, 10.0], look);
		let [x, y] = quad.corners[0]; // top left (-1, 1) turns to bottom left
		assert!((x - 9.0).abs() < 1e-5 && (y - 9.0).abs() < 1e-5, "{x}, {y}");
	}
}
