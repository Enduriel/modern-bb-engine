//! `highlightZoneOfControl(alliedFactions)` (`0x4b49a0` → `0x4b4aa0`): every tile the
//! player has seen that lies in a hostile zone of control, next to a hostile it can
//! see there (within a level), unless a map object stands on it. Hostile is any faction
//! not in `alliedFactions`; nothing happens if that isn't an array.
//!
//! The original keeps the hostile factions as a 32-bit mask, so a faction id of 32 or
//! more counts as the id's remainder; ours are a [`FactionSet`] (see
//! `docs/differences.md`).

use squirrel::{SQVM, SqArray, SqStrongRef};

use super::Highlight;
use crate::native::tactical_entities::Units;
use crate::native::tactical_grid::faction_set::FactionSet;
use crate::native::tactical_grid::visibility::PLAYER;
use crate::native::tactical_grid::{Grid, TacticalGridState};

/// Highlight the tiles; whether `allied` was an array (and the map may have changed).
pub(super) fn highlight(grid: &Grid, units: &Units, vm: &SQVM, allied: &SqStrongRef) -> bool {
	let Some(allied) = allied.get_value::<SqArray>(vm) else { return false };
	let allied = FactionSet::of(&allied.iter::<i64>().collect::<Vec<_>>());
	let hostile_at = |g: &TacticalGridState, id: usize| {
		g.tiles[id]
			.occupant
			.as_ref()
			.and_then(|(e, _)| units.info(e.identity()))
			.is_some_and(|u| !allied.contains(u.faction))
	};
	let mut g = grid.borrow_mut();
	let marked: Vec<usize> = (0..g.tiles.len())
		.filter(|&id| {
			let t = &g.tiles[id];
			!matches!(t.occupant, Some((_, false)))
				&& t.vis_mask.contains(PLAYER)
				&& t.zoc.mask.has_outside(&allied)
				&& (0..6).filter_map(|d| g.neighbour(id, d)).any(|n| {
					let nt = &g.tiles[n];
					nt.vis_mask.contains(PLAYER) && (nt.level - t.level).abs() <= 1 && hostile_at(&g, n)
				})
		})
		.collect();
	for id in marked {
		g.tiles[id].highlight = Highlight::ZoneOfControl;
	}
	true
}
