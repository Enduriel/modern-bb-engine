//! The world map's moving layer, item by item: what a frame draws besides the tiles'
//! terrain and details. An entity's sprite or label, a cloud, a region's name, a
//! footprint, a wave, a particle or a bird is one [`Item`], holding everything its quads
//! are made from: equal items make equal quads, so a renderer that keeps the quads from
//! frame to frame (`crate::kept_sprites`) builds again only the items that changed.
//!
//! [`items`] lists a scene's items in the order the original pushes them (the order
//! ties in the sort keep), kind by kind; [`Item::quads`] makes one item's quads, each
//! with its layer and sort key; [`Item::shows`] is whether a view draws it at all.

use std::rc::Rc;

use bb_assets::SpriteRegistry;
use squirrel_bb::scene::{SceneLabel, WorldScene};

use crate::labels::label_quads;
use crate::sprite::Quad;
use crate::text::{TextRun, glyph_quads, text_scale};
use crate::world::{
	BIRD_ALPHA, BIRD_SHADOW_ALPHA, BIRD_SHADOW_DROP, BIRD_SHADOW_SCALE, CLOUD_REACH, CLOUD_SHADOW_ALPHA, CLOUD_SHADOW_DROP,
	CLOUD_SHADOW_SCALE, Layer, Look, REGION_TEXT_COLOR, REGION_TEXT_SCALE, SPRITE_REACH, WorldFonts, WorldView, sprite_quad, unlit,
};

/// The kinds of items, in the order a scene's are pushed (see [`items`]). Each kind's
/// items are matched with the last frame's on their own (`crate::kept_sprites`).
pub(crate) const KINDS: usize = 8;

/// One thing the moving layer draws, with everything its quads are made from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item {
	/// An entity's sprite at its anchor (the entity's position). Its look's scale has
	/// the zoom in it for a sprite that keeps its screen size (`setSpriteScaling`).
	Sprite { brush: Rc<str>, frame: u32, anchor: [f32; 2], offset: [f32; 2], look: Look, unlit: bool },
	/// A cloud and its shadow.
	Cloud { brush: &'static str, pos: [f32; 2], scale: f32, alpha: f32, color: u32, key: i64 },
	/// A region's name, at `pixel` world units per screen pixel.
	Region { text: Rc<[u8]>, pos: [f32; 2], scale: f32, rotation: f32, pixel: f32 },
	Footprint { brush: Rc<str>, pos: [f32; 2], scale: f32, alpha: f32 },
	Wave { brush: Rc<str>, pos: [f32; 2], alpha: f32 },
	Particle { brush: Rc<str>, pos: [f32; 2], color: u32, scale: f32, rotation: f32 },
	/// A bird and its shadow.
	Bird { brush: &'static str, pos: [f32; 2], key: i64, frame: u32 },
	/// An entity's label at its anchor, at `pixel` world units per screen pixel.
	Label { label: SceneLabel, anchor: [f32; 2], pixel: f32 },
}

/// One of an item's quads: its scene layer, its sort key (a higher key draws first),
/// the atlas it samples and the quad.
pub(crate) struct ItemQuad<'a> {
	pub layer: Layer,
	pub key: i64,
	pub texture: &'a str,
	pub quad: Quad,
}

impl Item {
	/// Its kind, `0..KINDS`, in push order.
	pub(crate) fn kind(&self) -> usize {
		match self {
			Item::Sprite { .. } => 0,
			Item::Cloud { .. } => 1,
			Item::Region { .. } => 2,
			Item::Footprint { .. } => 3,
			Item::Wave { .. } => 4,
			Item::Particle { .. } => 5,
			Item::Bird { .. } => 6,
			Item::Label { .. } => 7,
		}
	}

	/// The point the view culls it by and how far past the screen it may be (`None`:
	/// always drawn).
	pub(crate) fn reach(&self) -> Option<([f32; 2], f32)> {
		match self {
			Item::Sprite { anchor, .. } | Item::Label { anchor, .. } => Some((*anchor, SPRITE_REACH)),
			Item::Cloud { pos, .. } => Some((*pos, CLOUD_REACH)),
			Item::Region { .. } => None,
			Item::Footprint { pos, .. } | Item::Wave { pos, .. } | Item::Particle { pos, .. } | Item::Bird { pos, .. } => {
				Some((*pos, SPRITE_REACH))
			}
		}
	}

	/// Whether `view` draws it: its point within the screen grown by its reach (the
	/// screen's edges included).
	pub(crate) fn shows(&self, view: &WorldView) -> bool {
		let Some(([x, y], reach)) = self.reach() else { return true };
		let [left, right, bottom, top] = view.bounds(reach);
		x >= left && x <= right && y >= bottom && y <= top
	}

	/// Its quads, in push order, onto `out`.
	pub(crate) fn quads<'a>(&self, sprites: &'a SpriteRegistry, fonts: WorldFonts<'a>, out: &mut Vec<ItemQuad<'a>>) {
		// `key`: the sort key without the brush's depth bias, which is added here.
		let mut push = |layer: Layer, key: i64, brush: &str, frame: u32, anchor: [f32; 2], offset: [f32; 2], look: Look| {
			let (texture, depth_bias, quad) = sprite_quad(sprites, brush, frame, anchor, offset, look)?;
			out.push(ItemQuad { layer, key: key + i64::from(depth_bias), texture, quad });
			Some(out.len() - 1)
		};
		match self {
			Item::Sprite { brush, frame, anchor, offset, look, unlit: dark } => {
				if let Some(at) = push(Layer::Entities, anchor[1] as i64, brush, *frame, *anchor, *offset, *look) {
					out[at].quad.flags = unlit(*dark);
				}
			}
			// Each a shadow and the cloud over it (`0x4e3ed0`). The shadow's key is 2000
			// more than the cloud's, so it draws first.
			Item::Cloud { brush, pos, scale, alpha, color, key } => {
				let [_, r, g, b] = color.to_be_bytes();
				let shadow = Look {
					scale: scale * CLOUD_SHADOW_SCALE,
					alpha: alpha * CLOUD_SHADOW_ALPHA * 255.0,
					tint: [0; 3],
					..Look::PLAIN
				};
				let below = [pos[0], pos[1] + CLOUD_SHADOW_DROP * scale];
				push(Layer::Effects, key + 2000, brush, 0, below, [0.0; 2], shadow);
				let look = Look { scale: *scale, alpha: alpha * 255.0, tint: [r, g, b], ..Look::PLAIN };
				push(Layer::Effects, *key, brush, 0, *pos, [0.0; 2], look);
			}
			Item::Region { text, pos, scale, rotation, pixel } => {
				let Some(font) = fonts.region else { return };
				// Text the bitmap font can't draw is shaped in the outline fonts.
				let decoded = crate::fonts::decode::engine_text(text);
				let shaped = match &decoded {
					crate::fonts::decode::EngineText::Bitmap(_) => None,
					crate::fonts::decode::EngineText::Shaped(text) => match fonts.shaper {
						Some(shaper) => Some(shaper.borrow_mut().shape(font.name(), text, None).clone()),
						None => return,
					},
				};
				let bytes: &[u8] = match &decoded {
					crate::fonts::decode::EngineText::Bitmap(bytes) => bytes,
					crate::fonts::decode::EngineText::Shaped(_) => &[],
				};
				let (width, height) = match &shaped {
					Some(shaped) => (shaped.width, shaped.height),
					None => (font.font.text_width(bytes) as f32, font.font.cell_height as f32),
				};
				let run = TextRun {
					text: bytes,
					font,
					pos: *pos,
					// Centred both ways.
					offset: [-width / 2.0, height / 2.0],
					scale: text_scale(*pixel, scale * REGION_TEXT_SCALE, true),
					rotation: *rotation,
					color: REGION_TEXT_COLOR,
				};
				// Region names are text nodes in the details layer whose key stays 0
				// (`0x4f0350`). A bitmap glyph samples the font's atlas.
				let quads: Vec<(&str, Quad)> = match &shaped {
					Some(shaped) => crate::text::shaped_quads(&run, shaped),
					None => glyph_quads(&run).into_iter().map(|(_, quad)| (font.texture, quad)).collect(),
				};
				out.extend(quads.into_iter().map(|(texture, quad)| ItemQuad { layer: Layer::Details, key: 0, texture, quad }));
			}
			// Key 0 in the details layer, after the region names (`0x4e49a0`).
			Item::Footprint { brush, pos, scale, alpha } => {
				let look = Look { scale: *scale, alpha: *alpha, ..Look::PLAIN };
				push(Layer::Details, 0, brush, 0, *pos, [0.0; 2], look);
			}
			// Key 0 in the effects layer, under the clouds.
			Item::Wave { brush, pos, alpha } => {
				let look = Look { alpha: *alpha, ..Look::PLAIN };
				push(Layer::Effects, 0, brush, 0, *pos, [0.0; 2], look);
			}
			// Key 0 in the effects layer, after the waves.
			Item::Particle { brush, pos, color, scale, rotation } => {
				let [a, r, g, b] = color.to_be_bytes();
				let look = Look { scale: *scale, alpha: f32::from(a), rotation: *rotation, tint: [r, g, b], ..Look::PLAIN };
				push(Layer::Effects, 0, brush, 0, *pos, [0.0; 2], look);
			}
			// Sorted by its key among the effects: white at alpha 0xa8 over its shadow, the
			// same frame at alpha 0x10, 1.25 times as large and offset 150 down (an offset
			// scales with its sprite).
			Item::Bird { brush, pos, key, frame } => {
				let shadow = Look {
					scale: BIRD_SHADOW_SCALE,
					alpha: f32::from(BIRD_SHADOW_ALPHA),
					tint: [0; 3],
					..Look::PLAIN
				};
				push(Layer::Effects, *key, brush, *frame, *pos, [0.0, -BIRD_SHADOW_DROP], shadow);
				let look = Look { alpha: f32::from(BIRD_ALPHA), ..Look::PLAIN };
				push(Layer::Effects, *key, brush, *frame, *pos, [0.0; 2], look);
			}
			// Above everything.
			Item::Label { label, anchor, pixel } => {
				let Some(font) = fonts.label else { return };
				out.extend(
					label_quads(label, *anchor, *pixel, font, fonts.shaper)
						.into_iter()
						.map(|(texture, quad)| ItemQuad { layer: Layer::Labels, key: anchor[1] as i64, texture, quad }),
				);
			}
		}
	}
}

/// `scene`'s items at `view` (whose zoom the labels, the region names and the sprites
/// that keep their screen size are made for), kind by kind in push order: the shown
/// entities' visible sprites, the clouds, the region names, the footprints, the waves,
/// the particles, the birds, then the shown entities' visible labels. Labels and
/// region names are listed only when `fonts` has their font.
pub(crate) fn items<'s>(scene: &'s WorldScene, view: &WorldView, fonts: WorldFonts<'_>) -> impl Iterator<Item = Item> + 's {
	let zoom = view.zoom;
	let pixel = 1.0 / zoom;
	let (label_font, region_font) = (fonts.label.is_some(), fonts.region.is_some());
	let shown = || scene.entities.iter().filter(|e| e.shown);
	let sprites = shown().flat_map(move |entity| {
		let anchor = [entity.pos.0 as f32, entity.pos.1 as f32];
		entity.sprites.iter().filter(|s| s.visible && !s.brush.is_empty()).map(move |sprite| {
			// `setSpriteScaling(name, false)`: the sprite keeps its screen size at any
			// zoom (the settlement banner).
			let zoom_scale = if sprite.scaling { 1.0 } else { 1.0 / zoom };
			let [_, r, g, b] = sprite.color.to_be_bytes();
			Item::Sprite {
				brush: sprite.brush.clone(),
				frame: sprite.frame,
				anchor,
				offset: [sprite.offset.0 as f32, sprite.offset.1 as f32],
				look: Look {
					scale: sprite.scale as f32 * zoom_scale,
					flipped: sprite.flipped,
					alpha: (sprite.color >> 24) as f32,
					rotation: sprite.rotation,
					tint: [r, g, b],
				},
				unlit: sprite.ignore_ambient,
			}
		})
	});
	let clouds = scene.clouds.iter().map(|c| Item::Cloud {
		brush: c.brush,
		pos: [c.pos.0, c.pos.1],
		scale: c.scale,
		alpha: c.alpha,
		color: c.color,
		key: c.key,
	});
	let regions = scene.region_texts.iter().filter(move |_| region_font).map(move |r| Item::Region {
		text: r.text.clone(),
		pos: [r.pos.0, r.pos.1],
		scale: r.scale,
		rotation: r.rotation,
		pixel,
	});
	let footprints = scene.footprints.iter().map(|f| Item::Footprint {
		brush: f.brush.clone(),
		pos: [f.pos.0, f.pos.1],
		scale: f.scale,
		alpha: f.alpha,
	});
	let waves = scene.waves.iter().map(|w| Item::Wave { brush: w.brush.clone(), pos: [w.pos.0, w.pos.1], alpha: w.alpha });
	let particles = scene.particles.iter().map(|p| Item::Particle {
		brush: p.brush.clone(),
		pos: [p.pos.0, p.pos.1],
		color: p.color,
		scale: p.scale,
		rotation: p.rotation,
	});
	let birds = scene.birds.iter().map(|b| Item::Bird { brush: b.brush, pos: [b.pos.0, b.pos.1], key: b.key, frame: b.frame });
	let labels = shown().filter(move |_| label_font).flat_map(move |entity| {
		let anchor = [entity.pos.0 as f32, entity.pos.1 as f32];
		entity
			.labels
			.iter()
			.filter(|l| l.visible && !l.text.is_empty())
			.map(move |label| Item::Label { label: label.clone(), anchor, pixel })
	});
	sprites.chain(clouds).chain(regions).chain(footprints).chain(waves).chain(particles).chain(birds).chain(labels)
}
