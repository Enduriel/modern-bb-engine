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
//! original keeps one quadtree per scene layer). Details sort back to front on the
//! original's key, `int(y) + depth_bias + z level` (`spawnDetail`, `0x4fb707`); ties
//! keep spawn order, and entities sort north to south, which is not yet checked.
//! Region names are text nodes in the details layer whose key stays 0 (`0x4f0350`),
//! so they draw over the details.

use std::collections::HashMap;

use bb_assets::{BrushFrame, SpriteRegistry};
use squirrel_bb::scene::WorldScene;

use crate::labels::{LabelFont, label_quads};
use crate::sprite::{Quad, SceneUniforms};
use crate::text::{TextRun, glyph_quads, text_scale};

/// The bitmap fonts world text draws in (`None` where the data lacks one).
#[derive(Clone, Copy, Default)]
pub struct WorldFonts<'a> {
	/// Entity labels (`gfx/fonts/cinzel_bold_20`).
	pub label: Option<LabelFont<'a>>,
	/// Region names (`gfx/fonts/cinzel_bold_100`, `0x47dcbc`).
	pub region: Option<LabelFont<'a>>,
}

/// How far a cloud's centre may be off screen and still show (cloud pictures are
/// large and scaled up to 1.25, their shadows 1.25 more and 500 units lower).
const CLOUD_REACH: f32 = 2000.0;

/// A cloud's shadow: 1.25 of its scale, a fifth of its alpha, 500 units times its
/// scale below it (`0x4e3ed0`).
const CLOUD_SHADOW_SCALE: f32 = 1.25;
const CLOUD_SHADOW_ALPHA: f32 = 0.2;
const CLOUD_SHADOW_DROP: f32 = -500.0;

/// Region names: white at alpha `0x66`, at 0.4 of the script's scale (`0x4f0350`).
const REGION_TEXT_COLOR: [u8; 4] = [255, 255, 255, 0x66];
const REGION_TEXT_SCALE: f32 = 0.4;

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

	/// The world rectangle on screen, `[left, right, bottom, top]`, grown by `margin`.
	fn bounds(&self, margin: f32) -> [f32; 4] {
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

/// One sprite to draw, before sorting.
struct Drawable<'a> {
	/// Scene layer, drawn in turn: tile terrain, details, entities.
	layer: Layer,
	/// Within a layer, a higher key draws first (further back).
	key: i64,
	/// Spawn order, to keep ties stable.
	order: usize,
	texture: &'a str,
	quad: Quad,
}

/// The atlases a frame uses, by texture path, in first-use order.
#[derive(Default)]
pub struct AtlasList {
	pub paths: Vec<String>,
	index: HashMap<String, usize>,
}

impl AtlasList {
	fn index_of(&mut self, path: &str) -> usize {
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
enum Layer {
	Terrain,
	Details,
	Entities,
	/// Clouds and their shadows (the original's effects layer).
	Effects,
	/// Entity labels, above everything.
	Labels,
}

/// A frame's quads in drawing order, and the atlases they index.
pub struct WorldQuads {
	pub quads: Vec<Quad>,
	pub atlases: AtlasList,
	/// Where the labels start in `quads` (they draw over the fog of war).
	pub labels_from: usize,
}

/// The quads that draw `scene` in `view`.
pub fn world_quads<'a>(
	scene: &'a WorldScene,
	sprites: &'a SpriteRegistry,
	view: &WorldView,
	fonts: WorldFonts<'a>,
) -> WorldQuads {
	// Tile sprites reach well past their hex (400 x 200 around a 180 x 120 cell).
	let [left, right, bottom, top] = view.bounds(400.0);
	let on_screen = |(x, y): (f64, f64)| {
		let (x, y) = (x as f32, y as f32);
		x >= left && x <= right && y >= bottom && y <= top
	};
	let mut drawables: Vec<Drawable> = Vec::new();
	// `key`: the sort key without the brush's depth bias, which is added here.
	let push = |drawables: &mut Vec<Drawable<'a>>,
	            layer: Layer,
	            key: i64,
	            brush: &str,
	            anchor: [f32; 2],
	            offset: [f32; 2],
	            look: Look| {
		let Some((sheet, sprite)) = sprites.get(brush) else {
			return;
		};
		let Some(frame) = sprite.frames.first() else {
			return;
		};
		// Offsets scale with the sprite (so an unscaled sprite keeps its screen
		// offset too).
		let anchor = [
			anchor[0] + (offset[0] + f32::from(sprite.offset_x)) * look.scale,
			anchor[1] + (offset[1] + f32::from(sprite.offset_y)) * look.scale,
		];
		let order = drawables.len();
		drawables.push(Drawable {
			layer,
			key: key + i64::from(sprite.depth_bias),
			order,
			texture: &sheet.sheet.texture,
			quad: frame_quad(frame, anchor, look),
		});
	};

	for tile in scene.tiles.iter().filter(|t| on_screen(t.pos)) {
		let anchor = [tile.pos.0 as f32, tile.pos.1 as f32];
		if !tile.brush.is_empty() {
			let key = anchor[1] as i64;
			push(
				&mut drawables,
				Layer::Terrain,
				key,
				&tile.brush,
				anchor,
				[0.0; 2],
				Look::PLAIN,
			);
		}
		for detail in &tile.details {
			let look = Look {
				scale: detail.scale as f32,
				flipped: detail.flipped,
				alpha: detail.alpha as f32,
				rotation: detail.rotation as f32,
				tint: [255; 3],
			};
			// The original's detail key: int(y) + depth bias + z level (`0x4fb707`).
			let key = anchor[1] as i64 + detail.z_level;
			push(
				&mut drawables,
				Layer::Details,
				key,
				&detail.brush,
				anchor,
				[0.0; 2],
				look,
			);
		}
	}
	for entity in scene
		.entities
		.iter()
		.filter(|e| e.shown && on_screen(e.pos))
	{
		let anchor = [entity.pos.0 as f32, entity.pos.1 as f32];
		for sprite in entity
			.sprites
			.iter()
			.filter(|s| s.visible && !s.brush.is_empty())
		{
			let offset = [sprite.offset.0 as f32, sprite.offset.1 as f32];
			// `setSpriteScaling(name, false)`: the sprite keeps its screen size at any
			// zoom (the settlement banner).
			let zoom_scale = if sprite.scaling { 1.0 } else { 1.0 / view.zoom };
			let look = Look {
				scale: sprite.scale as f32 * zoom_scale,
				flipped: sprite.flipped,
				alpha: (sprite.color >> 24) as f32,
				rotation: 0.0,
				tint: {
					let [_, r, g, b] = sprite.color.to_be_bytes();
					[r, g, b]
				},
			};
			push(
				&mut drawables,
				Layer::Entities,
				anchor[1] as i64,
				&sprite.brush,
				anchor,
				offset,
				look,
			);
		}
	}

	// Clouds: each a shadow and the cloud over it (`0x4e3ed0`). The shadow's key is
	// 2000 more than the cloud's, so it draws first.
	let [left, right, bottom, top] = view.bounds(CLOUD_REACH);
	for cloud in scene.clouds.iter().filter(|c| {
		let (x, y) = c.pos;
		x >= left && x <= right && y >= bottom && y <= top
	}) {
		let anchor = [cloud.pos.0, cloud.pos.1];
		let [_, r, g, b] = cloud.color.to_be_bytes();
		let shadow = Look {
			scale: cloud.scale * CLOUD_SHADOW_SCALE,
			alpha: cloud.alpha * CLOUD_SHADOW_ALPHA * 255.0,
			tint: [0; 3],
			..Look::PLAIN
		};
		let below = [anchor[0], anchor[1] + CLOUD_SHADOW_DROP * cloud.scale];
		push(
			&mut drawables,
			Layer::Effects,
			cloud.key + 2000,
			cloud.brush,
			below,
			[0.0; 2],
			shadow,
		);
		let look = Look {
			scale: cloud.scale,
			alpha: cloud.alpha * 255.0,
			tint: [r, g, b],
			..Look::PLAIN
		};
		push(
			&mut drawables,
			Layer::Effects,
			cloud.key,
			cloud.brush,
			anchor,
			[0.0; 2],
			look,
		);
	}

	if let Some(font) = fonts.region {
		let zoom = 1.0 / view.zoom;
		for region in &scene.region_texts {
			let text = region.text.as_bytes();
			let width = font.font.text_width(text) as f32;
			let height = font.font.cell_height as f32;
			let run = TextRun {
				text,
				font,
				pos: [region.pos.0, region.pos.1],
				// Centred both ways.
				offset: [-width / 2.0, height / 2.0],
				scale: text_scale(zoom, region.scale * REGION_TEXT_SCALE, true),
				rotation: region.rotation,
				color: REGION_TEXT_COLOR,
			};
			for (texture, quad) in glyph_quads(&run) {
				let order = drawables.len();
				drawables.push(Drawable {
					layer: Layer::Details,
					key: 0,
					order,
					texture,
					quad,
				});
			}
		}
	}

	// Footprints: key 0 in the details layer, after the region names (`0x4e49a0`).
	for footprint in scene
		.footprints
		.iter()
		.filter(|f| on_screen((f.pos.0.into(), f.pos.1.into())))
	{
		let look = Look {
			scale: footprint.scale,
			alpha: footprint.alpha,
			..Look::PLAIN
		};
		let anchor = [footprint.pos.0, footprint.pos.1];
		push(
			&mut drawables,
			Layer::Details,
			0,
			&footprint.brush,
			anchor,
			[0.0; 2],
			look,
		);
	}
	// Waves: key 0 in the effects layer, under the clouds.
	for wave in scene
		.waves
		.iter()
		.filter(|w| on_screen((w.pos.0.into(), w.pos.1.into())))
	{
		let look = Look {
			alpha: wave.alpha,
			..Look::PLAIN
		};
		push(
			&mut drawables,
			Layer::Effects,
			0,
			&wave.brush,
			[wave.pos.0, wave.pos.1],
			[0.0; 2],
			look,
		);
	}

	if let Some(font) = fonts.label {
		for entity in scene
			.entities
			.iter()
			.filter(|e| e.shown && on_screen(e.pos))
		{
			let anchor = [entity.pos.0 as f32, entity.pos.1 as f32];
			for label in entity
				.labels
				.iter()
				.filter(|l| l.visible && !l.text.is_empty())
			{
				for (texture, quad) in label_quads(label, anchor, 1.0 / view.zoom, font) {
					let order = drawables.len();
					drawables.push(Drawable {
						layer: Layer::Labels,
						key: anchor[1] as i64,
						order,
						texture,
						quad,
					});
				}
			}
		}
	}

	drawables.sort_by(|a, b| {
		a.layer
			.cmp(&b.layer)
			.then(b.key.cmp(&a.key))
			.then(a.order.cmp(&b.order))
	});
	let mut atlases = AtlasList::default();
	let labels_from = drawables
		.iter()
		.position(|d| d.layer == Layer::Labels)
		.unwrap_or(drawables.len());
	let quads = drawables
		.into_iter()
		.map(|d| Quad {
			texture: atlases.index_of(d.texture),
			..d.quad
		})
		.collect();
	WorldQuads {
		quads,
		atlases,
		labels_from,
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
	let (sin, cos) = look.rotation.to_radians().sin_cos();
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
		overlay: [0; 4],
		factor: 0.0,
		saturation: 1.0,
		flags: 0,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

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
