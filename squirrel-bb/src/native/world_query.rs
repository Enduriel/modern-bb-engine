//! World spatial queries (`World.getAllEntitiesAtPos` and
//! `World.getAllEntitiesAndOneLocationAtPos`), following the original's disassembly
//! (2026-10-13) and checked against probed results.
//!
//! The original answers these from two sources, in this order:
//!
//! 1. The `entity` scene layer (a depth-8 quadtree over the map), which holds the
//!    *rendered-top* entities, i.e. parties. It is queried with `radius + 50` (or
//!    `radius + 32` for the one-location variant), and every hit is returned as is.
//! 2. The id-ordered map of all entities, filtered to the *not*-rendered-top ones
//!    (locations, which call `setRenderedTop(false)` in `onInit`):
//!    * `getAllEntitiesAtPos` keeps those within `radius`;
//!    * `getAllEntitiesAndOneLocationAtPos` keeps only the first whose tile is the
//!      tile under `pos`.
//!
//! Distances are `f32`, as in the original.
//!
//! Known divergence: within the quadtree the original returns entities in quadtree
//! traversal order, which depends on each sprite's bounding rect and on insertion
//! history. We return them by ascending id instead. Locations, the common case,
//! come back in exactly the original order.

/// What a query needs to know about one live entity.
pub(crate) struct Candidate {
	pub id: i64,
	pub pos: (f64, f64),
	pub rendered_top: bool,
	/// Axial tile coords.
	pub tile: (i64, i64),
	/// `getVisibility()`: the multiplier on how far away it can be seen.
	pub visibility: f32,
}

/// Extra radius on the quadtree (rendered-top) part of `getAllEntitiesAtPos`.
pub(crate) const ALL_AT_POS_SLACK: f32 = 50.0;
/// Extra radius on the quadtree part of `getAllEntitiesAndOneLocationAtPos`.
pub(crate) const ONE_LOCATION_SLACK: f32 = 32.0;

/// The original's distance: `f32` deltas and sum, `sqrt` in `f64`, rounded back to `f32`.
/// Also `entity.getDistanceTo(other)`.
pub(crate) fn distance(a: (f64, f64), b: (f64, f64)) -> f32 {
	let dx = a.0 as f32 - b.0 as f32;
	let dy = a.1 as f32 - b.1 as f32;
	f64::from(dx * dx + dy * dy).sqrt() as f32
}

/// `getDirectionTo`/`getDirectionToPos`: the hex direction (0 N, 1 NE, 2 SE, 3 S,
/// 4 SW, 5 NW; world +Y is north) from `from` towards `to`.
pub(crate) fn direction6(from: (f64, f64), to: (f64, f64)) -> i64 {
	let dx = to.0 as f32 - from.0 as f32;
	let dy = to.1 as f32 - from.1 as f32;
	if dx.abs() < 20.0 {
		return if dy > 0.0 { 0 } else { 3 };
	}
	match (dx > 0.0, dy < 0.0) {
		(true, false) => 1,
		(true, true) => 2,
		(false, true) => 4,
		// dy == 0 counts as SW.
		(false, false) => if dy == 0.0 { 4 } else { 5 },
	}
}

/// `getDirection8To(pos)`: one of 8 directions (0 N, then clockwise), from the angle
/// `atan2(dx, dy)` in whole degrees, with the original's uneven bucket edges.
pub(crate) fn direction8(from: (f64, f64), to: (f64, f64)) -> i64 {
	let dx = to.0 as f32 - from.0 as f32;
	let dy = to.1 as f32 - from.1 as f32;
	let rad = f64::from(dx).atan2(f64::from(dy)) as f32;
	let mut deg = super::world_nav::cvtt_i32(f64::from(rad * 57.295_78_f32));
	if deg < 0 {
		deg += 360;
	}
	match deg {
		d if !(23..=336).contains(&d) => 0,
		d if d < 68 => 1,
		d if d < 112 => 2,
		d if d < 158 => 3,
		d if d < 202 => 4,
		d if d < 248 => 5,
		d if d < 292 => 6,
		_ => 7,
	}
}

/// The rendered-top entities within `radius + slack` of `center`.
fn rendered_top_near(center: (f64, f64), radius: f32, slack: f32, cands: &[Candidate]) -> Vec<usize> {
	let reach = radius + slack;
	let mut hits: Vec<usize> = (0..cands.len())
		.filter(|&i| cands[i].rendered_top && distance(center, cands[i].pos) <= reach)
		.collect();
	hits.sort_by_key(|&i| cands[i].id);
	hits
}

/// The candidates in id order, skipping the rendered-top ones.
fn ground_by_id(cands: &[Candidate]) -> Vec<usize> {
	let mut ids: Vec<usize> = (0..cands.len()).filter(|&i| !cands[i].rendered_top).collect();
	ids.sort_by_key(|&i| cands[i].id);
	ids
}

/// `getAllEntitiesAtPos(center, radius)`: indices into `cands`, in result order.
pub(crate) fn all_at_pos(center: (f64, f64), radius: f64, cands: &[Candidate]) -> Vec<usize> {
	let radius = radius as f32;
	let mut out = rendered_top_near(center, radius, ALL_AT_POS_SLACK, cands);
	out.extend(
		ground_by_id(cands)
			.into_iter()
			.filter(|&i| distance(center, cands[i].pos) <= radius),
	);
	out
}

/// `getAllEntitiesVisibleAtPos(center, radius)`: like [`all_at_pos`] (quadtree reach
/// `radius + 32`), keeping only entities with a non-zero visibility that are within
/// `visibility * radius`.
pub(crate) fn all_visible_at_pos(center: (f64, f64), radius: f64, cands: &[Candidate]) -> Vec<usize> {
	let radius = radius as f32;
	let seen = |i: usize| {
		let vis = cands[i].visibility;
		vis != 0.0 && distance(center, cands[i].pos) <= vis * radius
	};
	let mut out: Vec<usize> = rendered_top_near(center, radius, ONE_LOCATION_SLACK, cands)
		.into_iter()
		.filter(|&i| seen(i))
		.collect();
	out.extend(ground_by_id(cands).into_iter().filter(|&i| seen(i)));
	out
}

/// `getEntityAtTile(coords)`: the first entity by id that isn't rendered-top (a
/// location) on axial tile `tile`.
pub(crate) fn entity_at_tile(tile: (i64, i64), cands: &[Candidate]) -> Option<usize> {
	ground_by_id(cands).into_iter().find(|&i| cands[i].tile == tile)
}

/// `getAllEntitiesAndOneLocationAtPos(center, radius)`, where `center_tile` is the
/// axial tile under `center`.
pub(crate) fn all_and_one_location_at_pos(
	center: (f64, f64),
	radius: f64,
	center_tile: (i64, i64),
	cands: &[Candidate],
) -> Vec<usize> {
	let mut out = rendered_top_near(center, radius as f32, ONE_LOCATION_SLACK, cands);
	out.extend(entity_at_tile(center_tile, cands));
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn cand(id: i64, x: f64, y: f64, top: bool, tile: (i64, i64)) -> Candidate {
		Candidate {
			id,
			pos: (x, y),
			rendered_top: top,
			tile,
			visibility: 1.0,
		}
	}

	#[test]
	fn visible_scales_by_each_entitys_visibility() {
		let mut cands = vec![
			cand(1, 120.0, 0.0, true, (0, 0)),
			cand(2, 90.0, 0.0, false, (0, 0)),
			cand(3, 50.0, 0.0, false, (0, 0)),
		];
		cands[0].visibility = 1.5; // seen from 150 at radius 100
		cands[1].visibility = 0.5; // only from 50
		cands[2].visibility = 0.0; // never
		assert_eq!(all_visible_at_pos((0.0, 0.0), 100.0, &cands), [0]);
	}

	#[test]
	fn directions_follow_the_original_buckets() {
		let o = (1000.0, 1000.0);
		let at = |dx: f64, dy: f64| (o.0 + dx, o.1 + dy);
		// Six-way: a 20px vertical corridor, then the four diagonals by quadrant.
		assert_eq!(direction6(o, at(10.0, 50.0)), 0);
		assert_eq!(direction6(o, at(-19.0, -50.0)), 3);
		assert_eq!(direction6(o, at(0.0, 0.0)), 3);
		assert_eq!(direction6(o, at(180.0, 60.0)), 1);
		assert_eq!(direction6(o, at(180.0, 0.0)), 1);
		assert_eq!(direction6(o, at(180.0, -60.0)), 2);
		assert_eq!(direction6(o, at(-180.0, -60.0)), 4);
		assert_eq!(direction6(o, at(-180.0, 0.0)), 4);
		assert_eq!(direction6(o, at(-180.0, 60.0)), 5);
		// Eight-way: 0 is north, clockwise.
		assert_eq!(direction8(o, at(0.0, 100.0)), 0);
		assert_eq!(direction8(o, at(100.0, 100.0)), 1);
		assert_eq!(direction8(o, at(100.0, 0.0)), 2);
		assert_eq!(direction8(o, at(100.0, -100.0)), 3);
		assert_eq!(direction8(o, at(0.0, -100.0)), 4);
		assert_eq!(direction8(o, at(-100.0, -100.0)), 5);
		assert_eq!(direction8(o, at(-100.0, 0.0)), 6);
		assert_eq!(direction8(o, at(-100.0, 100.0)), 7);
		// Edge: 22.x degrees is still north, 23 is NE.
		let t22 = 22.5_f64.to_radians().tan();
		assert_eq!(direction8(o, at(100.0 * t22, 100.0)), 0);
		let t23 = 23.5_f64.to_radians().tan();
		assert_eq!(direction8(o, at(100.0 * t23, 100.0)), 1);
	}

	#[test]
	fn parties_first_then_locations_by_id() {
		// The shape probed from the original: the party first, then locations by id
		// regardless of distance.
		let c = (0.0, 0.0);
		let cands = vec![
			cand(20429, 254.5, 0.0, false, (1, 0)),
			cand(21388, 0.0, 0.0, true, (0, 0)),
			cand(19180, 360.0, 0.0, false, (2, 0)),
			cand(20423, 240.0, 0.0, false, (3, 0)),
		];
		let ids = |v: Vec<usize>| v.into_iter().map(|i| cands[i].id).collect::<Vec<_>>();
		assert_eq!(ids(all_at_pos(c, 300.0, &cands)), [21388, 20423, 20429]);
		assert_eq!(ids(all_at_pos(c, 600.0, &cands)), [21388, 19180, 20423, 20429]);
	}

	#[test]
	fn parties_get_the_extra_reach_locations_do_not() {
		let cands = vec![cand(1, 140.0, 0.0, true, (0, 0)), cand(2, 140.0, 0.0, false, (0, 0))];
		assert_eq!(all_at_pos((0.0, 0.0), 100.0, &cands), [0]);
		assert_eq!(all_at_pos((0.0, 0.0), 89.0, &cands), Vec::<usize>::new());
	}

	#[test]
	fn one_location_is_the_first_on_the_centre_tile() {
		let cands = vec![
			cand(5, 0.0, 0.0, false, (4, 4)),
			cand(3, 900.0, 0.0, false, (4, 4)),
			cand(9, 10.0, 0.0, false, (1, 1)),
			cand(7, 120.0, 0.0, true, (2, 2)),
		];
		assert_eq!(all_and_one_location_at_pos((0.0, 0.0), 100.0, (4, 4), &cands), [3, 1]);
		assert_eq!(all_and_one_location_at_pos((0.0, 0.0), 80.0, (0, 0), &cands), Vec::<usize>::new());
	}
}
