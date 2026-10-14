//! Which battle tiles have something on them that a tree or boulder in front of them
//! fades for. The original's object draw (`0x4c30c0`, disassembled) looks one
//! to three rows behind an object with `setSpriteOcclusion` sprites for a tile that the
//! player sees (`0x4b4940`: the player's bit of its visibility mask) and that holds
//! (`0x4d9ab0`):
//! - a unit that prevents occlusion (`setPreventOcclusion`, which every actor turns on;
//!   `+0x188` of the tile's entity);
//! - items on the ground (`IsContainingItems`, tile `+0x28`);
//! - a step of the movement path on show (tile `+0x27`, set with the footsteps by
//!   `0x4c8b60` and cleared with them by `0x4c8e10`).
//!
//! A unit out of the player's sight behind a tree doesn't fade it. The renderer reads the
//! answer from [`SceneTacticalTile::unveils`](crate::scene::tactical::SceneTacticalTile).

use crate::scene::tactical::TacticalScene;

/// Mark `scene`'s tiles: `preventers` are the tiles of the units that prevent occlusion,
/// `items` the tiles with items on them (indices into `scene.tiles`).
pub(crate) fn mark(scene: &mut TacticalScene, preventers: &[usize], items: &[usize]) {
	let path = scene.path.iter().map(|s| s.tile);
	for i in preventers.iter().copied().chain(items.iter().copied()).chain(path) {
		if let Some(t) = scene.tiles.get_mut(i) {
			t.unveils = t.visible;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::mark;
	use crate::scene::tactical::{SceneTacticalTile, TacticalScene};

	fn tile(col: i64, visible: bool) -> SceneTacticalTile {
		SceneTacticalTile {
			col,
			row: 0,
			pos: (0.0, 0.0),
			level: 0,
			terrain: 0,
			subtype: 0,
			blend_priority: 0,
			brush: String::new(),
			color: 0,
			visible,
			unveils: false,
			highlight: None,
			details: Vec::new(),
		}
	}

	#[test]
	fn only_what_the_player_sees_unveils() {
		let mut scene = TacticalScene { tiles: vec![tile(0, true), tile(1, false), tile(2, true), tile(3, true)], ..Default::default() };
		mark(&mut scene, &[0, 1], &[2]);
		let unveils: Vec<bool> = scene.tiles.iter().map(|t| t.unveils).collect();
		assert_eq!(unveils, [true, false, true, false]);
	}
}
