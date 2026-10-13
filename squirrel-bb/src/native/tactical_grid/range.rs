//! The battle map's in-range walk (see [`hex_range`](crate::native::hex_range)).

use super::TacticalGridState;

/// The tiles within `max` of `centre` (the centre excluded), in the original's order.
pub(crate) fn tiles_in_range(g: &TacticalGridState, centre: usize, max: i64) -> Vec<usize> {
	let t = &g.tiles[centre];
	crate::native::hex_range::tiles_in_range((t.q, t.r), max, g.tiles.len(), |q, r| g.idx_checked(q, r))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn covers_the_disk_once_without_the_centre() {
		let g = TacticalGridState::for_test(32, 32);
		let centre = g.idx(16, 16).unwrap();
		let got = tiles_in_range(&g, centre, 3);
		assert_eq!(got.len(), 36);
		let mut sorted = got.clone();
		sorted.sort();
		sorted.dedup();
		assert_eq!(sorted.len(), 36);
		assert!(!got.contains(&centre));
		// Sector 0 first: the tile straight in direction 0.
		assert_eq!(got[0], g.neighbour(centre, 0).unwrap());
	}
}
