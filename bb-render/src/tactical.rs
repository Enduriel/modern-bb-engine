//! Drawing a battle map from a [`TacticalScene`], as the original's tile node does
//! (`0x557b00`; its geometry from the scene setup, `0x4cec0e`).
//!
//! Tiles are 160 × 80 hexes in columns 120 apart, world y up. They draw in axial order:
//! rows (`r`) from the back, and within a row columns from the right. Each tile is
//! built up layer by layer, `40` units a level, up to the camera's level:
//! - below the tile's own level, a wall: its terrain's socket (180 × 100), shaded
//!   darker towards its foot. Where the camera cuts a taller tile off, the cut is
//!   capped with `tile_inside` instead;
//! - at its level, the top (170 × 110, its terrain brush), tinted by `tile.setColor`
//!   and shaded by how far below the camera it is. Before the top, `terrain_shadow_N`,
//!   `_NW` and `_NE` where the neighbour behind is lower; the top hides the part on the
//!   tile itself, so they only show on the ground behind.
//!
//! After the top come the transitions, always between a tile and its three back
//! neighbours. Where the tile's terrain blends over a neighbour's (a higher blend
//! priority at the same level, or the neighbour's wall rising behind a tile below the
//! camera), the tile's own border piece for that edge is drawn on the neighbour. Where
//! the neighbour's terrain blends over the tile's, the neighbour's piece is drawn on the
//! tile. Either way it lands on whichever of the two was drawn first. Last comes the
//! highlighter's mark, if the tile has one (`0x55ac88`).
//!
//! The details and the units and objects on the map are scene nodes of their own, drawn
//! after all the tiles.

use bb_assets::SpriteRegistry;
use squirrel_bb::scene::tactical::{
	SceneEffect, SceneOverlay, ScenePathStep, ScenePool, SceneRainSplat, SceneTacticalEntity, SceneTacticalTile,
	TacticalScene,
};

use crate::sprite::Quad;
use crate::world::{
	AtlasList, CLOUD_REACH, CLOUD_SHADOW_ALPHA, CLOUD_SHADOW_DROP, CLOUD_SHADOW_SCALE, Look, WorldQuads, WorldView,
	frame_quad,
};

/// A hex's height (they are 160 wide).
const HEX_H: f32 = 80.0;
/// A tile top's quad.
const TILE_W: f32 = 170.0;
const TILE_H: f32 = 110.0;
/// How far below the hex a level's quads start.
const TILE_DROP: f32 = 10.0;
/// A socket's (wall's) quad.
const SOCKET_W: f32 = 180.0;
const SOCKET_H: f32 = 100.0;
/// World units a level raises a tile.
pub const LEVEL_H: f32 = 40.0;
/// How much darker each level below the camera is drawn.
const LEVEL_SHADE: f32 = 0.1;
/// A wall's top edge only darkens below this many levels under the camera.
const WALL_TOP_GRACE: f32 = 2.6;
/// The depth of field's strength (`sprite_dof.vs`, `DOF_STRENGTH`).
pub const DOF_STRENGTH: f32 = 0.25;
/// The saturation of a tile the player doesn't see now, and of everything on it: its
/// details and what stands there (the original's tile, detail and entity draws,
/// `0x4b94bf`, `0x4bc8df`, `0x4c3379`, by the tile's player visibility bit).
const FOGGED_SATURATION: f32 = 0.33;
/// The layer the highlighter's mark is drawn at: above any camera level, so never
/// blurred (`0x55ad76`).
const HIGHLIGHT_LEVEL: u32 = 3;
/// The cap where the camera cuts a taller tile off, and the wall without a socket.
const TILE_INSIDE: &str = "tile_inside";
const DEFAULT_SOCKET: &str = "socket_earth";

/// The back neighbours, as `(direction, axial step)` (`Const.Direction`: N 0, NE 1,
/// SE 2, S 3, SW 4, NW 5).
const N: (i64, (i64, i64)) = (0, (0, 1));
const NE: (i64, (i64, i64)) = (1, (1, 0));
const NW: (i64, (i64, i64)) = (5, (-1, 1));

/// The edge facing back the other way.
fn opposite(direction: i64) -> i64 {
	(direction + 3) % 6
}

/// A tile's axial coordinates.
fn axial(tile: &SceneTacticalTile) -> (i64, i64) {
	(tile.col, tile.row - tile.col.div_euclid(2))
}

/// The tile at axial `(q, r)`, if on the map.
fn tile_at(scene: &TacticalScene, q: i64, r: i64) -> Option<&SceneTacticalTile> {
	let (col, row) = (q, r + q.div_euclid(2));
	(col >= 0 && col < scene.width && row >= 0 && row < scene.height)
		.then(|| &scene.tiles[(row * scene.width + col) as usize])
}

/// `color`'s RGB times `factor` (truncated, as the original's bytes), alpha kept.
fn shade(color: u32, factor: f32) -> [u8; 4] {
	let [a, r, g, b] = color.to_be_bytes();
	let f = |c: u8| (f32::from(c) / 255.0 * factor * 255.0) as u8;
	[f(r), f(g), f(b), a]
}

/// What a quad of the tile pass is drawn with.
#[derive(Clone, Copy)]
struct Paint {
	/// The upper and lower edges' colours.
	upper: [u8; 4],
	lower: [u8; 4],
	saturation: f32,
	level: u32,
}

impl Paint {
	fn flat(color: [u8; 4], saturation: f32, level: u32) -> Paint {
		Paint {
			upper: color,
			lower: color,
			saturation,
			level,
		}
	}
}

struct Builder<'a> {
	sprites: &'a SpriteRegistry,
	atlases: AtlasList,
	quads: Vec<Quad>,
}

impl Builder<'_> {
	/// `brush`'s first frame stretched over the rectangle `[left, right] × [bottom, top]`
	/// (the tiles' and walls' fixed geometry).
	fn rect(&mut self, brush: &str, [left, right, bottom, top]: [f32; 4], paint: Paint) {
		let Some((sheet, sprite)) = self.sprites.get(brush) else {
			return;
		};
		let Some(frame) = sprite.frames.first() else {
			return;
		};
		let [u0, u1, v0, v1] = frame.uv;
		let (upper, lower) = (paint.upper, paint.lower);
		self.quads.push(Quad {
			texture: self.atlases.index_of(&sheet.sheet.texture),
			corners: [[left, top], [right, top], [right, bottom], [left, bottom]],
			uv: [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
			color: upper,
			corner_colors: (upper != lower).then_some([upper, upper, lower, lower]),
			overlay: [0; 4],
			factor: 0.0,
			saturation: paint.saturation,
			flags: 0,
			level: paint.level,
		});
	}

	/// `brush` placed at `anchor` by its own frame (shadows, border pieces).
	fn sprite(&mut self, brush: &str, anchor: [f32; 2], paint: Paint) {
		let Some((sheet, sprite)) = self.sprites.get(brush) else {
			return;
		};
		let Some(frame) = sprite.frames.first() else {
			return;
		};
		let [r, g, b, a] = paint.upper;
		let look = Look {
			tint: [r, g, b],
			alpha: f32::from(a),
			..Look::PLAIN
		};
		self.quads.push(Quad {
			texture: self.atlases.index_of(&sheet.sheet.texture),
			saturation: paint.saturation,
			level: paint.level,
			..frame_quad(frame, anchor, look)
		});
	}

	/// A brush's own offset (`[0, 0]` for an unknown brush).
	fn brush_offset(&self, brush: &str) -> [f32; 2] {
		self.sprites
			.get(brush)
			.map_or([0.0; 2], |(_, s)| [f32::from(s.offset_x), f32::from(s.offset_y)])
	}

	/// A detail or an entity's sprite: placed by the brush's offset and frame.
	fn placed(&mut self, brush: &str, anchor: [f32; 2], look: Look, saturation: f32, level: u32) {
		self.placed_frame(brush, 0, anchor, look, saturation, level);
	}

	/// As [`Builder::placed`], showing frame `frame` of the brush (its animation's; see
	/// `squirrel_bb::native::sprite_anim`).
	fn placed_frame(&mut self, brush: &str, frame: u32, anchor: [f32; 2], look: Look, saturation: f32, level: u32) {
		let Some((sheet, sprite)) = self.sprites.get(brush) else {
			return;
		};
		let Some(frame) = sprite.frames.get(frame as usize).or(sprite.frames.first()) else {
			return;
		};
		let anchor = [
			anchor[0] + f32::from(sprite.offset_x) * look.scale,
			anchor[1] + f32::from(sprite.offset_y) * look.scale,
		];
		self.quads.push(Quad {
			texture: self.atlases.index_of(&sheet.sheet.texture),
			saturation,
			level,
			..frame_quad(frame, anchor, look)
		});
	}
}

/// One tile's walls, shadows, top and transitions.
fn draw_tile(b: &mut Builder, scene: &TacticalScene, tile: &SceneTacticalTile) {
	let camera_level = scene.camera.level.clamp(0, 3);
	let level = tile.level;
	let (px, py) = (tile.pos.0 as f32, tile.pos.1 as f32);
	// The hex's base, from which the original lays out its quads.
	let base = py - HEX_H / 2.0;
	let saturation = if tile.visible { 1.0 } else { FOGGED_SATURATION };
	let (q, r) = axial(tile);
	let neighbour = |(_, (dq, dr)): (i64, (i64, i64))| {
		tile_at(scene, q + dq, r + dr).filter(|t| !t.brush.is_empty())
	};
	let transition = scene.transitions.get(&tile.brush);
	for i in 0..=level.min(camera_level) {
		let lift = LEVEL_H * i as f32 - TILE_DROP;
		let bottom = base + lift;
		if i < level {
			let below = (camera_level - i) as f32;
			let paint = Paint {
				upper: shade(0xffff_ffff, 1.0 - (below - WALL_TOP_GRACE).max(0.0) * LEVEL_SHADE),
				lower: shade(0xffff_ffff, 1.0 - below * LEVEL_SHADE),
				saturation,
				level: i as u32,
			};
			if i == camera_level {
				let rect = [px - TILE_W / 2.0, px + TILE_W / 2.0, bottom, bottom + TILE_H];
				b.rect(TILE_INSIDE, rect, paint);
			} else {
				let socket = transition.and_then(|t| t.socket.as_deref()).unwrap_or(DEFAULT_SOCKET);
				let rect = [px - SOCKET_W / 2.0, px + SOCKET_W / 2.0, bottom, bottom + SOCKET_H];
				b.rect(socket, rect, paint);
			}
			continue;
		}
		// The top, at the tile's own level.
		let top_color = shade(tile.color, 1.0 - (camera_level - level) as f32 * LEVEL_SHADE);
		let paint = Paint::flat(top_color, saturation, i as u32);
		let centre = |x: f32, y: f32| [x, y - HEX_H / 2.0 + TILE_H / 2.0 + lift];
		for (side, brush) in [(N, "terrain_shadow_N"), (NW, "terrain_shadow_NW"), (NE, "terrain_shadow_NE")] {
			if neighbour(side).is_some_and(|n| n.level < level) {
				b.sprite(brush, centre(px, py), paint);
			}
		}
		let rect = [px - TILE_W / 2.0, px + TILE_W / 2.0, bottom, bottom + TILE_H];
		b.rect(&tile.brush, rect, paint);

		// This tile's terrain over a back neighbour's.
		for side in [N, NW, NE] {
			let Some(other) = neighbour(side) else { continue };
			let over = (other.level == level && other.blend_priority < tile.blend_priority)
				|| (other.level > level && level < camera_level);
			let Some(t) = transition.filter(|_| over) else { continue };
			if !t.blend_into_sockets && other.level != level {
				continue;
			}
			let edge = opposite(side.0);
			if let Some((_, piece)) = t.brushes.iter().find(|(d, _)| *d == edge) {
				b.sprite(piece, centre(other.pos.0 as f32, other.pos.1 as f32), paint);
			}
		}
		// A back neighbour's terrain over this tile's.
		for side in [N, NE, NW] {
			let Some(other) = neighbour(side) else { continue };
			if other.level != level || other.blend_priority <= tile.blend_priority {
				continue;
			}
			let piece = scene
				.transitions
				.get(&other.brush)
				.and_then(|t| t.brushes.iter().find(|(d, _)| *d == side.0));
			if let Some((_, piece)) = piece {
				let color = shade(other.color, 1.0 - (camera_level - level) as f32 * LEVEL_SHADE);
				b.sprite(piece, centre(px, py), Paint::flat(color, saturation, i as u32));
			}
		}
	}
	// The highlighter's mark (a zone of control, a skill's range, a target): the top's
	// rectangle again, in the mark's own colour, never shaded, desaturated or blurred.
	if let Some((brush, color)) = &tile.highlight {
		let bottom = base + LEVEL_H * level.min(camera_level) as f32 - TILE_DROP;
		let rect = [px - TILE_W / 2.0, px + TILE_W / 2.0, bottom, bottom + TILE_H];
		b.rect(brush, rect, Paint::flat(shade(*color, 1.0), 1.0, HIGHLIGHT_LEVEL));
	}
}

/// The alpha a faded sprite is drawn with (the original sets its colour's alpha to 60).
const FADED_ALPHA: f32 = 60.0 / 255.0;

/// What fades an object's sprites so that units behind it show (the object draw,
/// `0x4c30c0`, disassembled 2026-10-14). Each sprite with `setSpriteOcclusion(a, b, c)`
/// fades when:
/// - `HideTrees` is on and `c <= 0 <= b` (crowns and boulder tops, not trunks);
/// - otherwise a unit stands one row behind and the level it stands above the object
///   is within `c..=b`;
/// - or, with no unit there, a unit stands two rows behind (sprites with `a > 1`) or
///   three (`a > 2`), at most two levels above.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fade {
	None,
	/// Level difference of a unit one row behind.
	BehindOne(i64),
	BehindTwo,
	BehindThree,
	HideTrees,
}

impl Fade {
	fn of(scene: &TacticalScene, tile: &SceneTacticalTile, units: &std::collections::HashSet<usize>) -> Fade {
		if scene.hide_trees {
			return Fade::HideTrees;
		}
		let (q, r) = axial(tile);
		// "Behind" is up the map; with the camera flipped, down it.
		let step = if scene.camera.flipped { -1 } else { 1 };
		let unit_at = |rows: i64| {
			tile_at(scene, q, r + rows * step).filter(|t| units.contains(&((t.row * scene.width + t.col) as usize)))
		};
		if let Some(t) = unit_at(1) {
			return Fade::BehindOne(t.level - tile.level);
		}
		if unit_at(2).is_some_and(|t| t.level - tile.level <= 2) {
			return Fade::BehindTwo;
		}
		if unit_at(3).is_some_and(|t| t.level - tile.level <= 2) {
			return Fade::BehindThree;
		}
		Fade::None
	}

	/// The alpha factor for a sprite with occlusion `(a, b, c)`.
	fn alpha(self, (a, b, c): (i64, i64, i64)) -> f32 {
		let faded = a != 0
			&& match self {
				Fade::HideTrees => c <= 0 && 0 <= b,
				Fade::BehindOne(rise) => c <= rise && rise <= b,
				Fade::BehindTwo => a > 1,
				Fade::BehindThree => a > 2,
				Fade::None => false,
			};
		if faded { FADED_ALPHA } else { 1.0 }
	}
}

/// What lies and stands on a tile, besides its details.
#[derive(Default)]
struct OnTile<'a> {
	pools: Vec<&'a ScenePool>,
	standing: Vec<&'a SceneTacticalEntity>,
	steps: Vec<&'a ScenePathStep>,
	splats: Vec<&'a SceneRainSplat>,
}

/// A tile's details and what stands on it, each entity followed by its overlay (the
/// original draws a unit's bars with it, `0x4c3547`, so a unit in front covers the
/// bars of one behind), the overlay at `overlay_level`. On a tile the player can't see,
/// what isn't visible in the fog (units) isn't drawn (`0x4c30eb`), and the rest is
/// desaturated. Then the path's footsteps on the tile, and the rain's splats.
fn draw_dressing(b: &mut Builder, tile: &SceneTacticalTile, on: &OnTile, fade: Fade, overlay_level: u32) {
	let (x, y) = (tile.pos.0 as f32, tile.pos.1 as f32 + tile.level as f32 * LEVEL_H);
	let level = tile.level.max(0) as u32;
	let hidden = if tile.visible { 1.0 } else { FOGGED_SATURATION };
	for detail in &tile.details {
		let [a, r, g, b_] = detail.color.to_be_bytes();
		let look = Look {
			scale: detail.scale,
			flipped: detail.mirrored,
			alpha: f32::from(a),
			rotation: detail.rotation,
			tint: [r, g, b_],
		};
		let anchor = [x + detail.offset.0, y + detail.offset.1];
		b.placed_frame(&detail.brush, detail.frame, anchor, look, detail.saturation * hidden, level);
	}
	// The pools on the ground (`spawnPoolEffect`), under what stands there.
	for pool in &on.pools {
		let [a, r, g, b_] = pool.color.to_be_bytes();
		let look = Look {
			scale: pool.scale,
			flipped: pool.mirrored,
			alpha: f32::from(a),
			rotation: pool.rotation,
			tint: [r, g, b_],
		};
		b.placed(&pool.brush, [pool.pos.0, pool.pos.1], look, pool.saturation * hidden, level);
	}
	for entity in on.standing.iter().filter(|e| tile.visible || e.visible_in_fog) {
		draw_entity_sprites(b, entity, Some(fade), hidden, level);
		if let Some(overlay) = &entity.overlay {
			draw_overlay(b, entity, overlay, overlay_level);
		}
	}
	// A footstep sorts after what stands on its tile: the tile's list goes by key,
	// larger first (`0x55d170`), and a footstep's key is its brush's depth bias (-64)
	// where a unit's or an object's is 0. It draws on any tile at full colour, unlike a
	// unit (`0x4ba2f0`).
	for step in &on.steps {
		let [a, r, g, b_] = step.color.to_be_bytes();
		let look = Look {
			alpha: f32::from(a),
			tint: [r, g, b_],
			..Look::PLAIN
		};
		b.placed(&step.brush, [step.pos.0 as f32, step.pos.1 as f32], look, 1.0, level);
	}
	// A rain splat is attached to the tile under it, key -100: after the footsteps.
	for splat in &on.splats {
		let [a, r, g, b_] = splat.color.to_be_bytes();
		let look = Look {
			scale: splat.scale,
			alpha: f32::from(a),
			tint: [r, g, b_],
			..Look::PLAIN
		};
		b.placed_frame(&splat.brush, splat.frame, [splat.pos.0, splat.pos.1], look, 1.0, level);
	}
}

/// An entity's sprites, at `saturation` and `level`, each with its hit flash (its
/// `highlight` blended in by `highlight_factor`); with `fade`, the sprites with
/// occlusion take its alpha (the original writes it every frame: 60 faded, else 255).
fn draw_entity_sprites(b: &mut Builder, entity: &SceneTacticalEntity, fade: Option<Fade>, saturation: f32, level: u32) {
	let anchor = [entity.pos.0 as f32, entity.pos.1 as f32];
	for sprite in entity.sprites.iter().filter(|s| s.visible && !s.brush.is_empty()) {
		let [a, r, g, b_] = sprite.color.to_be_bytes();
		let alpha = match (fade, sprite.occlusion) {
			(Some(fade), Some(o)) if o.0 != 0 => fade.alpha(o) * 255.0,
			_ => f32::from(a),
		};
		let look = Look {
			scale: sprite.scale,
			flipped: sprite.flipped,
			alpha,
			rotation: sprite.rotation,
			tint: [r, g, b_],
		};
		let at = [anchor[0] + sprite.offset.0 * sprite.scale, anchor[1] + sprite.offset.1 * sprite.scale];
		let before = b.quads.len();
		b.placed_frame(&sprite.brush, sprite.frame, at, look, sprite.saturation * saturation, level);
		if sprite.highlight_factor != 0.0 {
			let [ha, hr, hg, hb] = sprite.highlight.to_be_bytes();
			for quad in &mut b.quads[before..] {
				quad.overlay = [hr, hg, hb, ha];
				quad.factor = sprite.highlight_factor;
			}
		}
	}
}

/// Where a unit's bars sit from it (the original's `0x65f654`, which the screenshot
/// fit matched to the pixel on three units), and each bar's drop, colour and its
/// lost-value twin's colour: head armour, body armour, hitpoints (`0x4b3250`).
const OVERLAY_OFFSET: [f32; 2] = [-1.0, 55.0];
const OVERLAY_BARS: [(f32, u32); 3] = [(0.0, 0xff70_8090), (-7.0, 0xff70_8090), (-14.0, 0xffb3_1f1f)];

/// Where the first status icon sits from the bars, and the step to the next: four to
/// a row, rows stacking upwards (`0x4b42f0`). Unlike the bars, the icons sit there
/// without their brushes' offset (the original's screenshot: 35 units lower than
/// with it, the `ui.brush` offset).
const OVERLAY_ICONS: [f32; 2] = [-30.0, 57.0];
const OVERLAY_ICON_STEP: f32 = 20.0;

/// A unit's bars (`createOverlay`), in the original's sprite order: the frame's
/// bottom; per value its lost-value twin, then its bar (`entityoverlay_bar_N`, N the
/// value in 24ths, none at 0); the frame's top; the status icons. They are lit by the
/// battle's light and lie under its clouds, as in the original's screenshot.
fn draw_overlay(b: &mut Builder, entity: &SceneTacticalEntity, overlay: &SceneOverlay, level: u32) {
	let anchor = [entity.pos.0 as f32 + OVERLAY_OFFSET[0], entity.pos.1 as f32 + OVERLAY_OFFSET[1]];
	let sprite = |b: &mut Builder, brush: &str, at: [f32; 2], color: u32| {
		let [a, r, g, b_] = color.to_be_bytes();
		let look = Look {
			tint: [r, g, b_],
			alpha: f32::from(a),
			..Look::PLAIN
		};
		b.placed(brush, at, look, 1.0, level);
	};
	let bar = |b: &mut Builder, steps: i32, drop: f32, color: u32| {
		if steps > 0 {
			let brush = format!("entityoverlay_bar_{}", steps.min(24));
			sprite(b, &brush, [anchor[0], anchor[1] + drop], color);
		}
	};
	// The frame, bars and icons show by the overlay's alpha; the twins by their own.
	let shown = |color: u32| {
		let alpha = ((color >> 24) as f32 * overlay.alpha) as u32;
		(color & 0x00ff_ffff) | (alpha.min(255) << 24)
	};
	sprite(b, "entityoverlay_bottom", anchor, shown(0xffff_ffff));
	for ((steps, lost), (drop, color)) in overlay.bars.iter().zip(&overlay.lost).zip(OVERLAY_BARS) {
		if let Some((before, lost_color)) = lost {
			bar(b, *before, drop, *lost_color);
		}
		bar(b, *steps, drop, shown(color));
	}
	sprite(b, "entityoverlay_top", anchor, shown(0xffff_ffff));
	for (i, icon) in overlay.icons.iter().enumerate() {
		let (column, row) = ((i % 4) as f32, (i / 4) as f32);
		let offset = b.brush_offset(icon);
		let at = [
			anchor[0] + OVERLAY_ICONS[0] + column * OVERLAY_ICON_STEP - offset[0],
			anchor[1] + OVERLAY_ICONS[1] + row * OVERLAY_ICON_STEP - offset[1],
		];
		sprite(b, icon, at, shown(0xffff_ffff));
	}
}

/// One of the battle's effects (see `battle_effects`). Icons, attacks and sprite effects
/// sit where they are without their brush's offset, as the original flags them.
fn draw_effect(b: &mut Builder, effect: &SceneEffect, level: u32) {
	let [a, r, g, b_] = effect.color.to_be_bytes();
	let look = Look {
		scale: effect.scale,
		flipped: effect.mirrored,
		alpha: f32::from(a),
		rotation: effect.rotation,
		tint: [r, g, b_],
	};
	let mut at = [effect.pos.0, effect.pos.1];
	if !effect.brush_offset {
		let offset = b.brush_offset(&effect.brush);
		at = [at[0] - offset[0] * effect.scale, at[1] - offset[1] * effect.scale];
	}
	b.placed(&effect.brush, at, look, effect.saturation, level);
}

/// The battle's clouds or fog over everything, each with its shadow, as the world's
/// (`0x4e3ed0`): back to front by key, a shadow's key 2000 more than its cloud's.
fn draw_clouds(b: &mut Builder, scene: &TacticalScene, view: &WorldView) {
	let [left, right, bottom, top] = view.bounds(CLOUD_REACH);
	let mut layer: Vec<(i64, usize, bool)> = Vec::new();
	for (i, cloud) in scene.clouds.iter().enumerate() {
		let (x, y) = cloud.pos;
		if x >= left && x <= right && y >= bottom && y <= top {
			layer.push((cloud.key + 2000, i, true));
			layer.push((cloud.key, i, false));
		}
	}
	layer.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
	// Effects draw sharp, at the camera's level.
	let level = scene.camera.level.clamp(0, 3) as u32;
	for (_, i, shadow) in layer {
		let cloud = &scene.clouds[i];
		let [_, r, g, b_] = cloud.color.to_be_bytes();
		let (anchor, look) = if shadow {
			let look = Look {
				scale: cloud.scale * CLOUD_SHADOW_SCALE,
				alpha: cloud.alpha * CLOUD_SHADOW_ALPHA * 255.0,
				tint: [0; 3],
				..Look::PLAIN
			};
			([cloud.pos.0, cloud.pos.1 + CLOUD_SHADOW_DROP * cloud.scale], look)
		} else {
			let look = Look {
				scale: cloud.scale,
				alpha: cloud.alpha * 255.0,
				tint: [r, g, b_],
				..Look::PLAIN
			};
			([cloud.pos.0, cloud.pos.1], look)
		};
		b.placed(cloud.brush, anchor, look, 1.0, level);
	}
}

/// The quads that draw `scene` in `view`: every tile in the original's order, then the
/// details and entities, then the weather.
pub fn tactical_quads<'a>(scene: &'a TacticalScene, sprites: &'a SpriteRegistry, view: &WorldView) -> WorldQuads {
	let [left, right, bottom, top] = view.bounds(SOCKET_W);
	let raise = LEVEL_H * 3.0;
	let mut order: Vec<&SceneTacticalTile> = scene
		.tiles
		.iter()
		.filter(|t| {
			let (x, y) = (t.pos.0 as f32, t.pos.1 as f32);
			!t.brush.is_empty() && x >= left && x <= right && y >= bottom - raise && y <= top + raise
		})
		.collect();
	order.sort_by_key(|t| {
		let (q, r) = axial(t);
		(std::cmp::Reverse(r), std::cmp::Reverse(q))
	});

	let mut on: std::collections::HashMap<usize, OnTile> = Default::default();
	for pool in &scene.pools {
		on.entry(pool.tile).or_default().pools.push(pool);
	}
	for step in &scene.path {
		on.entry(step.tile).or_default().steps.push(step);
	}
	// Flying units aren't on a tile: they draw after the tiles (see below).
	for entity in scene.entities.iter().filter(|e| !e.flying) {
		if let Some(tile) = entity.tile {
			on.entry(tile).or_default().standing.push(entity);
		}
	}
	// A splat off the map has no tile to be drawn with.
	for splat in &scene.rain_splats {
		if let Some(tile) = splat.tile {
			on.entry(tile).or_default().splats.push(splat);
		}
	}
	let nothing = OnTile::default();

	// The tiles units stand on, for what fades in front of them.
	let unit_tiles: std::collections::HashSet<usize> =
		scene.entities.iter().filter(|e| e.is_actor).filter_map(|e| e.tile).collect();
	let mut b = Builder {
		sprites,
		atlases: AtlasList::default(),
		quads: Vec::new(),
	};
	for tile in &order {
		draw_tile(&mut b, scene, tile);
	}
	// Overlays draw sharp, at the camera's level.
	let overlay_level = scene.camera.level.clamp(0, 3) as u32;
	for tile in &order {
		let index = (tile.row * scene.width + tile.col) as usize;
		let fade = Fade::of(scene, tile, &unit_tiles);
		draw_dressing(&mut b, tile, on.get(&index).unwrap_or(&nothing), fade, overlay_level);
	}
	// The marks on the tiles map objects stand on (`HighlightBlockedTiles`): sprites in
	// the effects layer, made at the battle's setup before anything else there (so over
	// the map, its highlights, footsteps and trees), sharp, and desaturated while the
	// player can't see their tile (`0x4cfe9b`, `0x4b94bf`).
	for mark in &scene.blocked_tiles {
		let seen = mark.tile.and_then(|t| scene.tiles.get(t)).is_none_or(|t| t.visible);
		let saturation = if seen { 1.0 } else { FOGGED_SATURATION };
		let [a, r, g, b_] = mark.color.to_be_bytes();
		let look = Look {
			alpha: f32::from(a),
			tint: [r, g, b_],
			..Look::PLAIN
		};
		b.placed(&mark.brush, [mark.pos.0, mark.pos.1], look, saturation, overlay_level);
	}
	// A unit in the air is in the original's effects layer, drawn after the tiles and
	// first in that layer (its box always stays in the layer's root): over everything
	// on the map, under the effects; its sprites only (no bars), not desaturated by a
	// tile the player can't see, with no occlusion fade (`0x4c3560`).
	for entity in scene.entities.iter().filter(|e| e.flying) {
		draw_entity_sprites(&mut b, entity, None, 1.0, overlay_level);
	}
	// Particle effects, then the weather over everything.
	for particle in &scene.particles {
		let [a, r, g, b_] = particle.color.to_be_bytes();
		let look = Look {
			scale: particle.scale,
			alpha: f32::from(a),
			rotation: particle.rotation,
			tint: [r, g, b_],
			..Look::PLAIN
		};
		let level = scene.camera.level.clamp(0, 3) as u32;
		b.placed(&particle.brush, [particle.pos.0, particle.pos.1], look, particle.saturation, level);
	}
	// Icons, attacks, sprite effects and projectiles, sharp at the camera's level.
	for effect in &scene.effects {
		draw_effect(&mut b, effect, overlay_level);
	}
	// The highlighter's icons (an area attack's, a blocked shot's), also in the effects
	// layer: at their position with the brush's offset, sharp, their colour's alpha
	// carrying the fade after a clear.
	for icon in &scene.overlay_icons {
		let [a, r, g, b_] = icon.color.to_be_bytes();
		let look = Look {
			alpha: f32::from(a),
			tint: [r, g, b_],
			..Look::PLAIN
		};
		b.placed(&icon.brush, [icon.pos.0, icon.pos.1], look, 1.0, overlay_level);
	}
	// The rain's drops (key -100 in the effects layer), then the weather's clouds.
	for drop in &scene.rain {
		let [a, r, g, b_] = drop.color.to_be_bytes();
		let look = Look {
			scale: drop.scale,
			alpha: f32::from(a),
			tint: [r, g, b_],
			..Look::PLAIN
		};
		b.placed(&drop.brush, [drop.pos.0, drop.pos.1], look, 1.0, overlay_level);
	}
	draw_clouds(&mut b, scene, view);
	let labels_from = b.quads.len();
	WorldQuads {
		quads: b.quads,
		atlases: b.atlases,
		labels_from,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn shading_keeps_alpha_and_truncates() {
		assert_eq!(shade(0x80ff_8040, 1.0), [255, 128, 64, 128]);
		assert_eq!(shade(0xffff_ffff, 0.7), [178, 178, 178, 255]);
	}

	#[test]
	fn hide_trees_fades_tops_not_trunks() {
		let top = (3, 2, -3);
		let trunk = (1, -1, -2);
		assert_eq!(Fade::HideTrees.alpha(top), FADED_ALPHA);
		assert_eq!(Fade::HideTrees.alpha(trunk), 1.0);
		// A unit behind on the same level fades the top only; two rows behind, sprites
		// with a > 1; three rows, a > 2.
		assert_eq!(Fade::BehindOne(0).alpha(top), FADED_ALPHA);
		assert_eq!(Fade::BehindOne(0).alpha(trunk), 1.0);
		assert_eq!(Fade::BehindTwo.alpha((1, 2, -3)), 1.0);
		assert_eq!(Fade::BehindTwo.alpha((2, 2, -3)), FADED_ALPHA);
		assert_eq!(Fade::BehindThree.alpha((2, 2, -3)), 1.0);
		assert_eq!(Fade::BehindThree.alpha(top), FADED_ALPHA);
		assert_eq!(Fade::None.alpha(top), 1.0);
	}

	use squirrel_bb::scene::tactical::{
		SceneOverlayIcon, SceneRainDrop, SceneRainSplat, SceneTacticalDetail, SceneTacticalSprite, SceneTacticalTile,
	};

	use crate::test_sprites::{frame_of, name_of, registry};

	fn tile(col: i64, visible: bool) -> SceneTacticalTile {
		SceneTacticalTile {
			col,
			row: 0,
			pos: (col as f64 * 120.0, 0.0),
			level: 0,
			terrain: 0,
			subtype: 0,
			blend_priority: 0,
			brush: "tile".into(),
			color: 0xffff_ffff,
			visible,
			unveils: false,
			highlight: None,
			details: Vec::new(),
		}
	}

	fn entity(id: i64, tile: usize, brush: &str, visible_in_fog: bool, flying: bool) -> SceneTacticalEntity {
		SceneTacticalEntity {
			id,
			pos: (tile as f64 * 120.0, 0.0),
			tile: Some(tile),
			flying,
			visible_in_fog,
			is_actor: !visible_in_fog,
			overlay: None,
			sprites: vec![SceneTacticalSprite {
				name: "body".into(),
				brush: brush.into(),
				color: 0xffff_ffff,
				scale: 1.0,
				rotation: 0.0,
				saturation: 1.0,
				highlight: 0xff60_0a0a,
				highlight_factor: if flying { 0.0 } else { 0.25 },
				visible: true,
				flipped: false,
				offset: (0.0, 0.0),
				ignore_ambient: false,
				occlusion: None,
				frame: 0,
			}],
		}
	}

	fn quads_of(scene: &TacticalScene) -> Vec<Quad> {
		let registry = registry();
		let view = WorldView {
			center: [120.0, 0.0],
			zoom: 1.0,
			width: 800,
			height: 600,
		};
		tactical_quads(scene, &registry, &view).quads
	}

	#[test]
	fn units_out_of_sight_are_not_drawn_and_objects_are_desaturated() {
		let scene = TacticalScene {
			width: 2,
			height: 1,
			tiles: vec![tile(0, false), tile(1, true)],
			entities: vec![entity(1, 0, "unit", false, false), entity(2, 0, "rock", true, false), entity(3, 1, "unit", false, false)],
			..TacticalScene::default()
		};
		let quads = quads_of(&scene);
		let units: Vec<_> = quads.iter().filter(|q| q.corners[0][0] < 50.0 && q.factor != 0.0).collect();
		assert!(units.len() == 1 && units[0].saturation < 0.5, "only the rock on the unseen tile, desaturated");
		let seen: Vec<_> = quads.iter().filter(|q| q.corners[0][0] > 50.0 && q.factor != 0.0).collect();
		assert_eq!(seen.len(), 1, "the unit in sight is drawn");
		assert_eq!((seen[0].overlay, seen[0].factor), ([0x60, 0x0a, 0x0a, 0xff], 0.25), "with its hit's flash");
	}

	#[test]
	fn a_flying_unit_draws_after_the_map_undesaturated() {
		let scene = TacticalScene {
			width: 2,
			height: 1,
			tiles: vec![tile(0, false), tile(1, true)],
			entities: vec![entity(1, 0, "unit", false, true), entity(2, 1, "rock", true, false)],
			..TacticalScene::default()
		};
		let quads = quads_of(&scene);
		let last = quads.last().expect("quads");
		assert_eq!(last.corners[0][0] < 50.0, true, "the flying unit is drawn last, over the rock");
		assert_eq!(last.saturation, 1.0, "not desaturated by the unseen tile it's over");
	}

	fn step(tile: usize, color: u32) -> ScenePathStep {
		ScenePathStep {
			tile,
			pos: (tile as f64 * 120.0, 0.0),
			brush: "step".into(),
			color,
		}
	}

	#[test]
	fn marks_footsteps_and_icons_draw_where_the_original_puts_them() {
		let mut marked = tile(0, false);
		marked.highlight = Some(("mark".into(), 0xc0ff_ffff));
		let scene = TacticalScene {
			width: 2,
			height: 1,
			tiles: vec![marked, tile(1, true)],
			entities: vec![entity(1, 1, "unit", false, false)],
			path: vec![step(0, 0xffe0_e5a2), step(1, 0xff77_7777)],
			overlay_icons: vec![SceneOverlayIcon {
				brush: "icon".into(),
				pos: (120.0, 0.0),
				color: 0x80ff_ffff,
				tile: None,
			}],
			..TacticalScene::default()
		};
		let quads = quads_of(&scene);
		let names: Vec<&str> = quads.iter().map(name_of).collect();
		let on_tile_0 = |q: &Quad| q.corners[0][0] < 60.0;
		// The mark comes right after its tile's top, at full colour on a tile out of sight,
		// at its own alpha, never blurred.
		let mark = names.iter().position(|&n| n == "mark").expect("the mark");
		assert_eq!(names[mark - 1], "tile", "{names:?}");
		assert!(quads[mark - 1].saturation < 0.5, "the tile is out of sight");
		let q = &quads[mark];
		assert_eq!((q.color, q.saturation, q.level), ([255, 255, 255, 0xc0], 1.0, HIGHLIGHT_LEVEL));
		// Footsteps: on the tile out of sight at full colour; on the unit's, after the unit.
		let steps: Vec<usize> = (0..quads.len()).filter(|&i| names[i] == "step").collect();
		assert_eq!(steps.len(), 2, "{names:?}");
		let (here, there) = if on_tile_0(&quads[steps[0]]) { (steps[0], steps[1]) } else { (steps[1], steps[0]) };
		assert_eq!((quads[here].color, quads[here].saturation), ([0xe0, 0xe5, 0xa2, 0xff], 1.0));
		assert_eq!(quads[there].color, [0x77, 0x77, 0x77, 0xff]);
		let unit = names.iter().position(|&n| n == "unit").expect("the unit");
		assert!(there > unit, "the footstep over the unit: {names:?}");
		// The icon is in the effects layer, after the map, at its colour's alpha.
		assert_eq!(names.last(), Some(&"icon"), "{names:?}");
		assert_eq!(quads.last().map(|q| q.color[3]), Some(0x80));
	}

	#[test]
	fn blocked_marks_cover_the_map_and_follow_their_tiles_sight() {
		let mark = |tile: usize| SceneOverlayIcon {
			brush: "mark".into(),
			pos: (tile as f32 * 120.0, 0.0),
			color: 0xa8ff_ffff,
			tile: Some(tile),
		};
		let scene = TacticalScene {
			width: 2,
			height: 1,
			tiles: vec![tile(0, false), tile(1, true)],
			entities: vec![entity(1, 0, "rock", true, false), entity(2, 1, "unit", false, true)],
			blocked_tiles: vec![mark(0), mark(1)],
			..TacticalScene::default()
		};
		let quads = quads_of(&scene);
		let names: Vec<&str> = quads.iter().map(name_of).collect();
		let marks: Vec<usize> = (0..quads.len()).filter(|&i| names[i] == "mark").collect();
		let rock = names.iter().position(|&n| n == "rock").expect("the rock");
		let flying = names.iter().position(|&n| n == "unit").expect("the flying unit");
		assert_eq!(marks.len(), 2, "{names:?}");
		assert!(marks.iter().all(|&m| rock < m && m < flying), "over the map, under a unit in the air: {names:?}");
		for &m in &marks {
			let seen = quads[m].corners[0][0] > 60.0;
			assert_eq!(quads[m].color[3], 0xa8);
			assert_eq!(quads[m].saturation, if seen { 1.0 } else { FOGGED_SATURATION }, "seen: {seen}");
		}
	}

	#[test]
	fn frames_splats_and_rain_draw_where_the_original_puts_them() {
		let mut ground = tile(0, true);
		ground.details.push(SceneTacticalDetail {
			brush: "anim".into(),
			flags: 0,
			mirrored: false,
			offset: (0.0, 0.0),
			rotation: 0.0,
			scale: 1.0,
			saturation: 1.0,
			color: 0xffff_ffff,
			frame: 2,
		});
		let mut unit = entity(1, 1, "anim", false, false);
		unit.sprites[0].frame = 1;
		let scene = TacticalScene {
			width: 2,
			height: 1,
			tiles: vec![ground, tile(1, true)],
			entities: vec![unit],
			path: vec![step(1, 0xffe0_e5a2)],
			rain_splats: vec![SceneRainSplat {
				tile: Some(1),
				brush: "anim".into(),
				pos: (120.0, 0.0),
				scale: 1.0,
				color: 0xc0ff_ffff,
				frame: 2,
			}],
			rain: vec![SceneRainDrop {
				brush: "drop".into(),
				pos: (60.0, 30.0),
				scale: 0.5,
				color: 0x60ff_ffff,
			}],
			..TacticalScene::default()
		};
		let quads = quads_of(&scene);
		let names: Vec<&str> = quads.iter().map(name_of).collect();
		let at = |name: &str, on_tile_1: bool, frame: u32| {
			(0..quads.len()).find(|&i| {
				names[i] == name && (quads[i].corners[0][0] > 60.0) == on_tile_1 && frame_of(&quads[i]) == frame
			})
		};
		let detail = at("anim", false, 2).expect("the detail, at its frame");
		let unit = at("anim", true, 1).expect("the unit's sprite, at its frame");
		let splat = at("anim", true, 2).expect("the splat, at its frame");
		let step = names.iter().position(|&n| n == "step").expect("the footstep");
		// On the splat's tile: the unit, its footstep, then the splat. Tile 0, in front,
		// comes after (its detail).
		assert!(unit < step && step < splat && splat < detail, "{names:?}");
		assert_eq!(quads[splat].color[3], 0xc0);
		assert_eq!(names.last(), Some(&"drop"), "the rain over the map: {names:?}");
		assert_eq!(quads.last().map(|q| q.color[3]), Some(0x60));
	}

	#[test]
	fn borders_face_back_the_other_way() {
		assert_eq!(opposite(N.0), 3);
		assert_eq!(opposite(NW.0), 2);
		assert_eq!(opposite(NE.0), 4);
	}
}
