//! Where a world entity draws among the rest of the world map (disassembled 2026-10-23,
//! checked against screenshots of the original).
//!
//! - **Its layer.** `World.spawnLocation` adds the entity to the scene's details layer
//!   (`World+0x1c`, `0x4f0260`), the layer of the map's decorations (trees, mountains,
//!   houses), so a settlement or location sorts among them by key; `World.spawnEntity`
//!   adds it to the entities layer (`World+0x20`, `0x4f0170`), drawn after the whole
//!   details layer: a party is over every settlement, location and tree. `RenderedTop`
//!   only decides the spatial queries (`world_query`), not the layer.
//! - **Its key** (the scene node's `+0x10`, a higher key drawn first). A node's
//!   `setPos` sets it to the new `y` truncated (`0x542070`, while the node has flag
//!   `0x200000`, which the world entity's constructor sets), then calls the world
//!   entity's own hook (vtable `+0x2c`, `0x4e7d20`): if the tile under it changed, the
//!   key is that minus 20. So a spawned entity (its tile none before) has `int(y) - 20`;
//!   a party moving within a tile `int(y)`, and `int(y) - 20` on the step into the next.
//!   A save keeps the key as it is (the record's Z) and a load puts it back.
//! - **Its sprites** draw at that key plus its `setZOffset` (`0x4e7ac0`), all of them
//!   (the brush's depth bias isn't added), except those whose scaling is off
//!   (`setSpriteScaling(name, false)`: a settlement's banner, a party's zoom banner, the
//!   selection ring), which draw 2000 lower (`0x4e7c5e`): over the details within 2000
//!   units south of the entity. The scene's key has the Z offset in it; the renderer
//!   lowers the unscaled sprites (`bb_render::world_items`).
//! - **Its labels** are child `Text` nodes, drawn in its layer right after it, at its
//!   key (without the Z offset) plus their own, which `addLabel` sets to the entity's Z
//!   offset less 2100 (`0x4e7571`, flag 8: relative to the parent's, `0x4a6740`). So a
//!   party is drawn over a settlement's name, and the fog of war over every label (the
//!   world state's `onRender` draws the scene, then `World.render` the fog, `0x4a24c0`).

use super::EntityRecord;

/// How much lower the key is on a step onto another tile.
const TILE_CHANGED: i64 = 20;

/// How much lower than its entity's a label's key is.
const LABEL: i64 = 2100;

/// A label's key from its entity's, made by `addLabel` while the entity's Z offset is
/// `z_offset`.
pub(super) fn label(z_offset: i64) -> i64 {
	z_offset - LABEL
}

/// `y` truncated as the original does, through its `f32` (`cvttss2si`).
fn truncated(y: f64) -> i64 {
	i64::from(y as f32 as i32)
}

/// The key of an entity spawned at `y`: its tile changed from none.
pub(super) fn spawned(y: f64) -> i64 {
	truncated(y) - TILE_CHANGED
}

/// `setPos(pos)` on `rec`, onto tile `tile`: the position, the tile and the key.
pub(super) fn set_pos(rec: &mut EntityRecord, pos: (f64, f64), tile: (i64, i64)) {
	let changed = (rec.q, rec.r) != tile;
	let key = truncated(pos.1) - if changed { TILE_CHANGED } else { 0 };
	if rec.pos != pos || rec.key != key {
		crate::native::world_scene_changes::note();
	}
	rec.pos = pos;
	(rec.q, rec.r) = tile;
	rec.key = key;
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn keys_truncate_through_f32() {
		assert_eq!(spawned(16740.0), 16720);
		assert_eq!(spawned(-0.5), -20);
		assert_eq!(truncated(16785.9), 16785);
		// Past f32's precision the f32 decides.
		assert_eq!(truncated(16_777_217.0), 16_777_216);
	}
}
