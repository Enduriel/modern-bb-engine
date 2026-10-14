//! `highlightRangeOfSkill(skill, actor)` (`0x4b4c20` → `0x4b4f20`): the tiles the skill
//! reaches from the actor's tile, and of those the ones it can be used on.
//!
//! It reads the skill's `m.MinRange`, `m.MaxRange`, `m.IsRanged` and `m.MaxRangeBonus`
//! directly (a `getMaxRange()` override doesn't count) and does nothing if the skill has
//! no `onVerifyTarget`. The tiles from `MinRange` to `MaxRange` (and some for height,
//! below) away, in the map's range order, then the actor's own if `MinRange` is 0, are
//! kept when:
//!
//! 1. the latest vision update saw them;
//! 2. a ranged skill (`IsRanged`, `MaxRange` over 1) reaches them: `MaxRange` plus the
//!    levels the actor stands above the tile, at most `MaxRangeBonus`; a melee one is at
//!    most a level up or down;
//! 3. no map object stands on them.
//!
//! `skill.onVerifyTarget(origin, tile)` then makes each a target, or just in range.

use squirrel::{SQVM, SqStrongRef};

use super::Highlight;
use crate::native::tactical_grid::{self, Grid, hex_distance};

/// Highlight the tiles; whether the skill could be asked (and the map may have changed).
pub(super) fn highlight(grid: &Grid, vm: &SQVM, skill: &SqStrongRef, origin: usize) -> Result<bool, String> {
	let has_verify = skill
		.get_member::<SqStrongRef>(vm, "onVerifyTarget")
		.is_some_and(|f| !matches!(f.get_value::<Option<i64>>(vm), Some(None)));
	let m = skill.get_member::<SqStrongRef>(vm, "m").ok_or("highlightRangeOfSkill: the skill has no m")?;
	let int = |name: &str| m.get_member::<i64>(vm, name).unwrap_or(0);
	if !has_verify {
		return Ok(false);
	}
	let (min, max, bonus) = (int("MinRange"), int("MaxRange"), int("MaxRangeBonus"));
	let ranged = m.get_member::<bool>(vm, "IsRanged").unwrap_or(false) && max > 1;
	let candidates: Vec<usize> = {
		let g = grid.borrow();
		let o = &g.tiles[origin];
		let extra = if ranged { o.level.min(bonus) } else { 0 };
		let mut tiles: Vec<usize> = tactical_grid::range::tiles_in_range(&g, origin, max + extra)
			.into_iter()
			.filter(|&t| hex_distance(o, &g.tiles[t]) >= min)
			.collect();
		if min == 0 {
			tiles.push(origin);
		}
		tiles
			.into_iter()
			.filter(|&id| {
				let t = &g.tiles[id];
				let climb = o.level - t.level;
				t.vis_stamp == g.vis_stamp
					&& if ranged { hex_distance(o, t) <= max + climb.max(0).min(bonus) } else { climb.abs() <= 1 }
					&& !matches!(t.occupant, Some((_, false)))
			})
			.collect()
	};
	let origin_tile = tactical_grid::tile_handle(grid, vm, origin).ok_or("highlightRangeOfSkill: no tile class")?;
	for id in candidates {
		let tile = tactical_grid::tile_handle(grid, vm, id).ok_or("highlightRangeOfSkill: no tile class")?;
		let target = skill
			.call_method_ret::<bool, _>(vm, "onVerifyTarget", (origin_tile.clone(), tile))
			.map_err(|e| format!("onVerifyTarget: {e}"))?
			.unwrap_or(false);
		grid.borrow_mut().tiles[id].highlight = if target { Highlight::Target } else { Highlight::InRange };
	}
	Ok(true)
}
