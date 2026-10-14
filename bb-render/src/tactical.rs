//! Drawing a battle map from a [`TacticalScene`]: tiles, their raised walls, the
//! transitions where terrains meet, and their details.
//!
//! Geometry (from the scripts and the picking masks, `tactical_grid::picking`): tiles
//! are 160 × 80 hexes in columns 120 apart, odd columns half a row up, with world y up
//! (rows grow north, so lower tiles are nearer). A tile at level `n` is drawn `40 n`
//! higher, with its terrain's socket (`TileTransition` socket) below for the wall.
//! Where a neighbour's terrain has a higher blend priority, that terrain's transition
//! piece for the direction facing it is drawn over the tile. Tiles draw back to front.
//!
//! First version, to be checked against screenshots of the original: the socket and
//! transition rules are read from the data, not yet the original's code.

use bb_assets::SpriteRegistry;
use squirrel_bb::scene::tactical::{SceneTacticalTile, TacticalScene};

use crate::sprite::Quad;
use crate::world::{AtlasList, Look, WorldQuads, WorldView, frame_quad};

/// World units a level raises a tile.
const LEVEL_H: f32 = 40.0;

/// Axial neighbour steps by direction (`Const.Direction`: N, NE, SE, S, SW, NW).
const DIRECTION_STEP: [(i64, i64); 6] = [(0, 1), (1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1)];

/// The tile at axial `(q, r)`, if on the map.
fn tile_at(scene: &TacticalScene, q: i64, r: i64) -> Option<&SceneTacticalTile> {
	let (col, row) = (q, r + q / 2);
	(col >= 0 && col < scene.width && row >= 0 && row < scene.height)
		.then(|| &scene.tiles[(row * scene.width + col) as usize])
}

/// The quads that draw `scene`'s terrain in `view`, back to front.
pub fn tactical_quads<'a>(
	scene: &'a TacticalScene,
	sprites: &'a SpriteRegistry,
	view: &WorldView,
) -> WorldQuads {
	let [left, right, bottom, top] = view.bounds(300.0);
	let mut order: Vec<&SceneTacticalTile> = scene
		.tiles
		.iter()
		.filter(|t| {
			let (x, y) = (t.pos.0 as f32, t.pos.1 as f32 + t.level as f32 * LEVEL_H);
			x >= left && x <= right && y >= bottom - LEVEL_H * 3.0 && y <= top
		})
		.collect();
	// Back to front: further north first; ties left to right.
	order.sort_by(|a, b| b.pos.1.total_cmp(&a.pos.1).then(a.col.cmp(&b.col)));

	let mut atlases = AtlasList::default();
	let mut quads = Vec::new();
	let push = |brush: &str,
	            anchor: [f32; 2],
	            look: Look,
	            quads: &mut Vec<Quad>,
	            atlases: &mut AtlasList| {
		let Some((sheet, sprite)) = sprites.get(brush) else {
			return;
		};
		let Some(frame) = sprite.frames.first() else {
			return;
		};
		let anchor = [
			anchor[0] + f32::from(sprite.offset_x) * look.scale,
			anchor[1] + f32::from(sprite.offset_y) * look.scale,
		];
		quads.push(Quad {
			texture: atlases.index_of(&sheet.sheet.texture),
			..frame_quad(frame, anchor, look)
		});
	};
	for tile in order {
		let (x, ground) = (tile.pos.0 as f32, tile.pos.1 as f32);
		let y = ground + tile.level as f32 * LEVEL_H;
		let transition = scene.transitions.get(&tile.brush);
		// The wall under a raised tile: its socket, once per level.
		if let Some(socket) = transition.and_then(|t| t.socket.as_deref()) {
			for level in 0..tile.level {
				push(
					socket,
					[x, ground + level as f32 * LEVEL_H],
					Look::PLAIN,
					&mut quads,
					&mut atlases,
				);
			}
		}
		let [_, r, g, b] = tile.color.to_be_bytes();
		let tinted = Look {
			tint: [r, g, b],
			..Look::PLAIN
		};
		push(&tile.brush, [x, y], tinted, &mut quads, &mut atlases);
		// Higher-priority neighbours bleed over the edge facing them.
		let (q, r) = (tile.col, tile.row - tile.col / 2);
		for (direction, (dq, dr)) in DIRECTION_STEP.iter().enumerate() {
			let Some(other) = tile_at(scene, q + dq, r + dr) else {
				continue;
			};
			if other.brush == tile.brush
				|| other.blend_priority <= tile.blend_priority
				|| other.level != tile.level
			{
				continue;
			}
			let Some(piece) = scene
				.transitions
				.get(&other.brush)
				.and_then(|t| t.brushes.iter().find(|(d, _)| *d == direction as i64))
			else {
				continue;
			};
			push(&piece.1, [x, y], tinted, &mut quads, &mut atlases);
		}
		for detail in &tile.details {
			let [a, r, g, b] = detail.color.to_be_bytes();
			let look = Look {
				scale: detail.scale,
				flipped: detail.mirrored,
				alpha: f32::from(a),
				rotation: detail.rotation,
				tint: [r, g, b],
			};
			push(
				&detail.brush,
				[x + detail.offset.0, y + detail.offset.1],
				look,
				&mut quads,
				&mut atlases,
			);
		}
	}
	let labels_from = quads.len();
	WorldQuads {
		quads,
		atlases,
		labels_from,
	}
}
