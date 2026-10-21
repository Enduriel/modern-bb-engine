//! `Tactical.getTileSquare` off the map. The original doesn't check: it reads the
//! column's tile list at `columns[x].tiles[y]` (`0x4d2a00`; each column its own
//! allocation, so nothing off the map aliases a tile on it), and a square left of the
//! map (`x = -1`) reads before the column array and crashes the game ("A critical
//! exception occured", probed 2026-10-21). Vanilla reaches that: `tactical_template.
//! addRoads`' north-south walk guards its west step with `roadY` instead of `roadX`, so
//! now and then its road walks off the west edge (once in about 1,100 soak battles).
//! Ours gives the nearest square on the map instead and logs a warning, so such a
//! battle starts (its road runs down the edge) where the original's game dies.
//! `isValidTileSquare` and everything inside the engine stay strictly checked.

use squirrel::{SQVM, SqStrongRef, SqTable};

use super::Grid;

/// The tile id for square `(x, y)`, or, off the map, the nearest square's (with a
/// warning); an error only when there is no map.
pub(super) fn square_or_nearest(grid: &Grid, vm: &SQVM, x: i64, y: i64) -> Result<usize, String> {
	let (id, at) = {
		let g = grid.borrow();
		if let Some(id) = g.idx(x, y) {
			return Ok(id);
		}
		if g.width <= 0 || g.height <= 0 {
			return Err(format!("getTileSquare: ({x}, {y}) is off the map (no map)"));
		}
		let at = (x.clamp(0, g.width - 1), y.clamp(0, g.height - 1));
		(g.idx(at.0, at.1).expect("clamped onto the map"), at)
	};
	warn(vm, &format!("Tactical.getTileSquare({x}, {y}) is off the map (the original crashes here); using ({}, {})", at.0, at.1));
	Ok(id)
}

/// Log `message` as the scripts' `logWarning` does.
fn warn(vm: &SQVM, message: &str) {
	if let Some(log) = SqTable::root(vm).get::<SqStrongRef>("logWarning") {
		let _ = log.call(vm, (message.to_string(),));
	}
}
