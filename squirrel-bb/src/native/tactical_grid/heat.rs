//! AI path heat: `Tactical.clearHeat()` and `Tactical.fillHeat(tile, range, mult)`, which
//! some AI agents (hyenas, goblin wolfriders) lay around their opponents so their paths
//! avoid them (the navigator's `HeatCost`). Disassembled.
//!
//! `fillHeat` floods out from the tile, depth first, and adds
//! `trunc((range − d + 1) · mult)` to each tile it reaches, `d` being the hex distance to
//! the tile. It expands only tiles closer than `range`, so it reaches the tiles within
//! `range` plus, always, the six neighbours, and never the tile itself. Nothing else
//! stops it (heights, occupants), so the order of the flood doesn't matter.

use squirrel::{Native, Number, SQVM, SqStrongRef};

use super::{Grid, TacticalGridState, TileHandle, hex_distance};

fn fill(g: &mut TacticalGridState, center: usize, range: i64, mult: f32) {
	for id in 0..g.tiles.len() {
		let d = hex_distance(&g.tiles[center], &g.tiles[id]);
		if d == 1 || (d >= 1 && d <= range) {
			g.tiles[id].heat += ((range - d + 1) as f32 * mult) as i32;
		}
	}
}

/// The two natives, for the `Tactical` table.
pub(super) fn natives(vm: &mut SQVM, grid: &Grid) -> Vec<(&'static str, SqStrongRef)> {
	let clear = {
		let g = grid.clone();
		vm.make_closure(move || g.borrow_mut().tiles.iter_mut().for_each(|t| t.heat = 0))
	};
	let fill_heat = {
		let g = grid.clone();
		vm.make_closure(move |tile: Native<TileHandle>, range: i64, mult: Number| {
			fill(&mut g.borrow_mut(), tile.0.id, range, mult.0 as f32);
		})
	};
	vec![("clearHeat", clear), ("fillHeat", fill_heat)]
}

#[cfg(test)]
mod tests {
	use super::{TacticalGridState, fill, hex_distance};

	#[test]
	fn heat_falls_off_with_distance_and_skips_the_centre() {
		let mut g = TacticalGridState::for_test(16, 16);
		let c = g.idx(8, 8).unwrap();
		fill(&mut g, c, 3, 1.0);
		assert_eq!(g.tiles[c].heat, 0);
		for t in &g.tiles {
			let d = hex_distance(&g.tiles[c], t);
			let want = if (1..=3).contains(&d) { (3 - d + 1) as i32 } else { 0 };
			assert_eq!(t.heat, want, "d = {d}");
		}
		// Range 0 still heats the neighbours (by 0 · mult here), and only them.
		let mut g = TacticalGridState::for_test(16, 16);
		fill(&mut g, c, -1, 2.0);
		let n: Vec<i32> = g.tiles.iter().filter(|t| hex_distance(&g.tiles[c], t) == 1).map(|t| t.heat).collect();
		assert_eq!(n, vec![-2; 6]);
		assert_eq!(g.tiles.iter().filter(|t| t.heat != 0).count(), 6);
	}
}
