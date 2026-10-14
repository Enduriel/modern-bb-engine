//! The blocked-tile highlights, shown with the `HighlightBlockedTiles` setting (the
//! battle's topbar toggle): every tile a map object stands on (a tree, a rock),
//! marked with the target overlay at alpha 0xa8 in the battle's effects layer
//! (disassembled):
//!
//! - `Tactical.createBlockedTileHighlights()` (`0x4cfd50`), once the battle's map is
//!   made: a mark on each such tile, column by column, shown if the setting is on;
//! - `Tactical.setBlockedTileHighlightsVisibility(on)` (`0x4cfff0`): show or hide them;
//! - `Tactical.clearBlockedTileHighlights()` (`0x4cffb0`): remove them.
//!
//! They are made once: an object that goes later (a tree felled) leaves its mark.

use crate::native::tactical_grid::TacticalGridState;
use crate::scene::tactical::SceneOverlayIcon;

const BRUSH: &str = "zone_target_overlay";
const COLOR: u32 = 0xa8ff_ffff;

#[derive(Default)]
pub(super) struct Blocked {
	tiles: Vec<usize>,
	shown: bool,
}

impl Blocked {
	/// `createBlockedTileHighlights()` on map `g`, shown or not.
	pub fn create(&mut self, g: &TacticalGridState, shown: bool) {
		for col in 0..g.width {
			for row in 0..g.height {
				let Some(id) = g.idx(col, row) else { continue };
				// An entity without `onMovementStart`: a map object, not an actor.
				if matches!(g.tiles[id].occupant, Some((_, false))) {
					self.tiles.push(id);
				}
			}
		}
		self.shown = shown;
	}

	/// `setBlockedTileHighlightsVisibility(on)`.
	pub fn show(&mut self, on: bool) {
		self.shown = on;
	}

	/// `clearBlockedTileHighlights()`.
	pub fn clear(&mut self) {
		self.tiles.clear();
	}

	/// The marks as drawn now (none while hidden).
	pub fn scene(&self, g: &TacticalGridState) -> Vec<SceneOverlayIcon> {
		if !self.shown {
			return Vec::new();
		}
		self.tiles
			.iter()
			.filter_map(|&id| g.tiles.get(id))
			.map(|t| {
				let (x, y) = t.world_pos();
				SceneOverlayIcon {
					brush: BRUSH.to_string(),
					pos: (x as f32, y as f32),
					color: COLOR,
				}
			})
			.collect()
	}
}
