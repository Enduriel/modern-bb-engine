//! World-map pixel layout, matching the original engine (probed 2026-10-13).
//!
//! Tiles are flat-topped hexes 240 px wide and 120 px tall, packed in odd-shifted
//! columns: column spacing 180 px, row spacing 120 px, odd columns 60 px lower.
//! Worldgen depends on both directions exactly: `buildLabels` averages tile `Pos`
//! values and maps the centroid back with `World.worldToTile`.

/// The centre of the tile at square `(col, row)`, in world pixels.
pub(crate) fn tile_pos(col: i64, row: i64) -> (f64, f64) {
	let x = 120 + 180 * col;
	let y = 60 + 120 * row + if col % 2 != 0 { 60 } else { 0 };
	(x as f64, y as f64)
}

/// `World.worldToTile`: the axial `(q, r)` of the tile containing world point `(x, y)`
/// on a `width` x `height` grid. Reproduces the original's rectangle-and-corner test in
/// `f32`, including its edge cases (verified on 16,799 sampled points):
/// x is clamped to >= 0; the corner test only runs when the base cell is on the map;
/// comparisons are strict; the result is clamped in axial space.
pub(crate) fn world_to_tile(x: f64, y: f64, width: i64, height: i64) -> (i64, i64) {
	let x = (x as f32).max(0.0);
	let y = y as f32;
	let mut col = (x / 180.0).floor() as i64;
	let xr = x - (col * 180) as f32;
	let odd = col % 2 != 0;
	let yo = y - if odd { 60.0 } else { 0.0 };
	let mut row = (yo / 120.0).floor() as i64;
	let yr = yo - (row * 120) as f32;
	let inside = (0..width).contains(&col) && (0..height).contains(&row);
	if xr < 60.0 && inside {
		if yr < 60.0 {
			if xr < 60.0 - yr {
				if !odd {
					row -= 1;
				}
				col -= 1;
			}
		} else if xr < yr - 60.0 {
			if odd {
				row += 1;
			}
			col -= 1;
		}
	}
	let q = col.clamp(0, width - 1);
	let r = row - col.div_euclid(2);
	let row = (r + q / 2).clamp(0, height - 1);
	(q, row - q / 2)
}

#[cfg(test)]
mod tests {
	use super::{tile_pos, world_to_tile};

	#[test]
	fn matches_points_probed_in_the_original() {
		assert_eq!(tile_pos(0, 0), (120.0, 60.0));
		assert_eq!(tile_pos(1, 0), (300.0, 120.0));
		assert_eq!(tile_pos(139, 169), (25140.0, 20400.0));
		let (w, h) = (140, 170);
		for ((x, y), want) in [
			((130.5, 60.25), (0, 0)),
			((210.0, 90.0), (1, 0)),
			((209.9, 90.1), (0, 0)),
			((300.0, 180.0), (1, 1)),
			((389.99, 150.0), (1, 0)),
			((180.0, 10.0), (1, 0)),  // top edge, odd column: no corner test off-map
			((-60.0, 60.0), (0, 0)),  // x clamped to 0
			((-60.0, 70.0), (0, 1)),
			((550.0, 130.0), (3, -1)), // on a diagonal edge: strict test keeps the right column
			((200.0, 140.0), (1, 0)),
		] {
			assert_eq!(world_to_tile(x, y, w, h), want, "({x}, {y})");
		}
	}
}
