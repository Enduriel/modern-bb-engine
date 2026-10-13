//! Drawing the world map from a [`WorldScene`]: tiles, their details and the
//! entities' sprites, as quads for the sprite program.
//!
//! First pass, to be checked against screenshots of the original:
//! - world units are pixels at zoom 1, y up (tile rows grow north);
//! - a frame's rect is in y-down pixels around the sprite's anchor, so its top edge
//!   goes up in the world, and the atlas is stored upside down (`v1` is the top);
//! - brush and sprite offsets are y up;
//! - draw order: tile terrain, then details by z level (back to front), then
//!   entities, each layer north to south.

use std::collections::HashMap;

use bb_assets::{BrushFrame, SpriteRegistry};
use squirrel_bb::scene::WorldScene;

use crate::sprite::{Quad, SceneUniforms};

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
	/// Draw layer: higher draws first (further back).
	layer: i64,
	/// World y of the anchor: higher (further north) draws first within a layer.
	y: f32,
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

/// Layer of tile terrain: behind every detail.
const TERRAIN_LAYER: i64 = 1000;
/// Layer of entities, between objects (-10) and locations (-20) for now.
const ENTITY_LAYER: i64 = -15;

/// The quads that draw `scene` in `view`, and the atlases they index.
pub fn world_quads(
	scene: &WorldScene,
	sprites: &SpriteRegistry,
	view: &WorldView,
) -> (Vec<Quad>, AtlasList) {
	// Tile sprites reach well past their hex (400 x 200 around a 180 x 120 cell).
	let [left, right, bottom, top] = view.bounds(400.0);
	let on_screen = |(x, y): (f64, f64)| {
		let (x, y) = (x as f32, y as f32);
		x >= left && x <= right && y >= bottom && y <= top
	};
	let mut drawables: Vec<Drawable> = Vec::new();
	let mut push = |layer: i64,
	                brush: &str,
	                anchor: [f32; 2],
	                offset: [f32; 2],
	                scale: f32,
	                flipped: bool,
	                alpha: f32| {
		let Some((sheet, sprite)) = sprites.get(brush) else {
			return;
		};
		let Some(frame) = sprite.frames.first() else {
			return;
		};
		let anchor = [
			anchor[0] + offset[0] + f32::from(sprite.offset_x),
			anchor[1] + offset[1] + f32::from(sprite.offset_y),
		];
		let order = drawables.len();
		drawables.push(Drawable {
			layer,
			y: anchor[1],
			order,
			texture: &sheet.sheet.texture,
			quad: frame_quad(frame, anchor, scale, flipped, alpha),
		});
	};

	for tile in scene.tiles.iter().filter(|t| on_screen(t.pos)) {
		let anchor = [tile.pos.0 as f32, tile.pos.1 as f32];
		if !tile.brush.is_empty() {
			push(
				TERRAIN_LAYER,
				&tile.brush,
				anchor,
				[0.0; 2],
				1.0,
				false,
				255.0,
			);
		}
		for detail in &tile.details {
			push(
				detail.z_level,
				&detail.brush,
				anchor,
				[0.0; 2],
				detail.scale as f32,
				detail.flipped,
				detail.alpha as f32,
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
			push(
				ENTITY_LAYER,
				&sprite.brush,
				anchor,
				offset,
				sprite.scale as f32,
				sprite.flipped,
				sprite.alpha as f32,
			);
		}
	}

	drawables.sort_by(|a, b| {
		b.layer
			.cmp(&a.layer)
			.then(b.y.total_cmp(&a.y))
			.then(a.order.cmp(&b.order))
	});
	let mut atlases = AtlasList::default();
	let quads = drawables
		.into_iter()
		.map(|d| Quad {
			texture: atlases.index_of(d.texture),
			..d.quad
		})
		.collect();
	(quads, atlases)
}

/// A frame drawn at `anchor` (world units, y up).
pub fn frame_quad(
	frame: &BrushFrame,
	anchor: [f32; 2],
	scale: f32,
	flipped: bool,
	alpha: f32,
) -> Quad {
	let [left, right, top, bottom] = frame.rect;
	let [mut u0, mut u1, v0, v1] = frame.uv;
	if flipped {
		std::mem::swap(&mut u0, &mut u1);
	}
	let (x0, x1) = (anchor[0] + left * scale, anchor[0] + right * scale);
	let (y_top, y_bottom) = (anchor[1] - top * scale, anchor[1] - bottom * scale);
	Quad {
		texture: 0,
		corners: [[x0, y_top], [x1, y_top], [x1, y_bottom], [x0, y_bottom]],
		uv: [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
		color: [255, 255, 255, alpha.clamp(0.0, 255.0) as u8],
		overlay: [0; 4],
		factor: 0.0,
		saturation: 1.0,
		flags: 0,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_frame_hangs_from_its_anchor_with_the_atlas_flipped() {
		let frame = BrushFrame {
			name: String::new(),
			uv: [0.1, 0.2, 0.3, 0.4],
			rect: [-10.0, 10.0, -20.0, 5.0],
		};
		let quad = frame_quad(&frame, [100.0, 50.0], 1.0, false, 255.0);
		// Top edge 20 px above the anchor, bottom edge 5 px below.
		assert_eq!(
			quad.corners,
			[[90.0, 70.0], [110.0, 70.0], [110.0, 45.0], [90.0, 45.0]]
		);
		assert_eq!(quad.uv[0], [0.1, 0.4]);
		assert_eq!(quad.uv[2], [0.2, 0.3]);
	}

	#[test]
	fn flipping_mirrors_the_texture_not_the_quad() {
		let frame = BrushFrame {
			name: String::new(),
			uv: [0.1, 0.2, 0.3, 0.4],
			rect: [-10.0, 10.0, -20.0, 5.0],
		};
		let quad = frame_quad(&frame, [0.0, 0.0], 2.0, true, 128.0);
		assert_eq!(quad.corners[0], [-20.0, 40.0]);
		assert_eq!(quad.uv[0], [0.2, 0.4]);
		assert_eq!(quad.color[3], 128);
	}
}
