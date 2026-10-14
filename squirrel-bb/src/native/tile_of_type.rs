//! `World.findNextTileOfType(tile, dist, type, result)`, as the original computes it
//! (disassembled at `0x4f3850`; checked against 40 probed answers and the earlier
//! successive-answer probe below).
//!
//! It range-queries the tiles within `dist` of the centre, centre included, in the grid
//! helper's walk order (see [`hex_range`](super::hex_range)). A tile qualifies if it
//! has the type and all six of its neighbours are on the map and have the type too: an
//! interior tile of that terrain. The nearest qualifying tile by hex distance wins; only
//! a strictly nearer tile replaces the pick, so ties go to the earlier one in walk order.
//!
//! The original also sums a mismatch penalty over a second, nested neighbour loop, but
//! that loop reads the candidate's own neighbours again, which the first loop already
//! required to match. The penalty is therefore always zero and is not modelled.

use super::hex_range::tiles_in_range;

/// The axial step for each direction (`Const.Direction`, N=0 .. NW=5).
const STEP: [(i64, i64); 6] = [(0, 1), (1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1)];

fn distance((aq, ar): (i64, i64), (bq, br): (i64, i64)) -> i64 {
	let (dq, dr) = (aq - bq, ar - br);
	(dq.abs() + dr.abs() + (dq + dr).abs()) / 2
}

/// The tile `findNextTileOfType` answers, or `None`. `idx` maps axial coords to a tile
/// id (`None` off the map, ids below `n_tiles`), `coords` the reverse, and `type_of`
/// gives a tile's `Type`.
pub(crate) fn next_tile_of_type(
	centre: (i64, i64),
	dist: i64,
	ty: i64,
	n_tiles: usize,
	idx: impl Fn(i64, i64) -> Option<usize>,
	coords: impl Fn(usize) -> (i64, i64),
	type_of: impl Fn(usize) -> i64,
) -> Option<usize> {
	let centre_id = idx(centre.0, centre.1)?;
	let walk = std::iter::once(centre_id).chain(tiles_in_range(centre, dist, n_tiles, &idx));
	let mut best = 9000;
	let mut found = None;
	for id in walk {
		if type_of(id) != ty {
			continue;
		}
		let at = coords(id);
		let d = distance(at, centre);
		if d >= best {
			continue;
		}
		let interior = STEP
			.iter()
			.all(|(dq, dr)| idx(at.0 + dq, at.1 + dr).is_some_and(|n| type_of(n) == ty));
		if interior {
			best = d;
			found = Some(id);
		}
	}
	found
}

#[cfg(test)]
mod tests {
	use super::next_tile_of_type;

	/// Axial coords of square `(col, row)`.
	fn ax(col: i64, row: i64) -> (i64, i64) {
		(col, row - col / 2)
	}

	/// The probe the earlier lattice model was fitted to: every tile one type, and each
	/// answer's type changed before the next query. Changing a tile disqualifies its
	/// neighbours, so successive answers land two apart, which looked like a lattice.
	#[test]
	fn successive_answers_match_the_original() {
		let (w, h) = (100i64, 100i64);
		let mut types = vec![1i64; (w * h) as usize];
		let idx = |q: i64, r: i64| {
			let row = r + q / 2;
			(q >= 0 && q < w && row >= 0 && row < h).then(|| (row * w + q) as usize)
		};
		let coords = |id: usize| {
			let (col, row) = (id as i64 % w, id as i64 / w);
			(col, row - col / 2)
		};
		let mut got = Vec::new();
		while let Some(id) = next_tile_of_type(ax(60, 60), 4, 1, types.len(), idx, coords, |i| types[i]) {
			got.push(format!("{},{}", id as i64 % w, id as i64 / w));
			types[id] = 2;
		}
		assert_eq!(
			got.join(" "),
			"60,60 60,62 62,61 62,59 60,58 58,59 58,61 60,64 62,63 64,62 64,60 64,58 62,57 60,56 58,57 56,58 56,60 56,62 58,63"
		);
	}
}
