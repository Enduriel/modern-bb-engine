//! The world map's tile searches, in the original engine's exact visiting order
//! (recovered by probing the original game, 2026-10-13). Worldgen keeps the first
//! match, so the order decides the map.

fn hex_distance(aq: i64, ar: i64, bq: i64, br: i64) -> i64 {
	let (dq, dr) = (aq - bq, ar - br);
	(dq.abs() + dr.abs() + (dq + dr).abs()) / 2
}

/// What a flood callback tells the traversal about a tile it entered (the original's
/// codes; its code 0, "skip", is a tile the flood's `enter` test rejects).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
	/// 1: mark it and keep flooding through it.
	Expand = 1,
	/// 3: mark it but do not flood through it.
	Stop = 3,
}

/// The tiles a flood has marked.
trait Marks {
	fn insert(&mut self, t: (i64, i64));
	fn contains(&self, t: (i64, i64)) -> bool;
}

impl Marks for std::collections::HashSet<(i64, i64)> {
	fn insert(&mut self, t: (i64, i64)) {
		std::collections::HashSet::insert(self, t);
	}

	fn contains(&self, t: (i64, i64)) -> bool {
		std::collections::HashSet::contains(self, &t)
	}
}

/// Marks for a flood that never enters a tile more than `reach` steps (on either axial
/// axis) from `center`: a dense square window around it.
struct Window {
	center: (i64, i64),
	reach: i64,
	marked: Vec<bool>,
}

impl Window {
	fn new(center: (i64, i64), reach: i64) -> Window {
		let side = (2 * reach + 1) as usize;
		Window {
			center,
			reach,
			marked: vec![false; side * side],
		}
	}

	/// The tile's slot, or `None` outside the window.
	fn slot(&self, (q, r): (i64, i64)) -> Option<usize> {
		let (dq, dr) = (
			q - self.center.0 + self.reach,
			r - self.center.1 + self.reach,
		);
		let side = 2 * self.reach + 1;
		(0..side).contains(&dq).then_some(())?;
		(0..side).contains(&dr).then(|| (dq * side + dr) as usize)
	}
}

impl Marks for Window {
	fn insert(&mut self, t: (i64, i64)) {
		let slot = self
			.slot(t)
			.expect("a flood only enters tiles inside its window");
		self.marked[slot] = true;
	}

	fn contains(&self, t: (i64, i64)) -> bool {
		self.slot(t).is_some_and(|slot| self.marked[slot])
	}
}

/// The world map's recursive flood (mode 1 of the original's tile traversal, as used by
/// `World.findNextRegion`; recovered from the disassembly, 2026-10-13). Starting at the
/// centre (never offered to `visit`), each tile tries its neighbours in the order
/// N, S, NE, NW, SE, SW; an unmarked neighbour that `enter` accepts is offered to
/// `visit`, and the flood recurses into it, marking it on entry. A tile entered with
/// [`Visit::Stop`] is not expanded. Rejected tiles stay unmarked, so another path may
/// offer them again.
///
/// The recursion runs on an explicit stack: each frame is a tile and the next of its six
/// directions to try, which visits tiles in exactly the recursive order.
fn flood(
	center: (i64, i64),
	marks: &mut impl Marks,
	enter: impl Fn(i64, i64) -> bool,
	mut visit: impl FnMut(i64, i64) -> Visit,
) {
	const ORDER: [(i64, i64); 6] = [(0, 1), (0, -1), (1, 0), (-1, 1), (1, -1), (-1, 0)];
	marks.insert(center);
	let mut stack = vec![(center, 0)];
	while let Some((t, dir)) = stack.last_mut() {
		let Some(&(dq, dr)) = ORDER.get(*dir) else {
			stack.pop();
			continue;
		};
		*dir += 1;
		let n = (t.0 + dq, t.1 + dr);
		if !enter(n.0, n.1) || marks.contains(n) {
			continue;
		}
		match visit(n.0, n.1) {
			Visit::Expand => {
				marks.insert(n);
				stack.push((n, 0));
			}
			Visit::Stop => marks.insert(n),
		}
	}
}

/// The largest `dist` [`flood_last`] marks in a dense window (`(2 * dist + 1)^2` cells);
/// farther floods fall back to a hash set.
const WINDOW_MAX_DIST: i64 = 127;

/// `World.findNextRegion`: flood within `dist` of the centre; `test` picks matching
/// tiles, which are not flooded through. The **last** match wins (the original's
/// callback overwrites its result and carries on). `in_grid` must not have side effects:
/// it is consulted after the distance check, so it may run fewer times than in the
/// original (`test` runs exactly as often, in the same order).
pub(crate) fn flood_last<T>(
	center: (i64, i64),
	dist: i64,
	in_grid: impl Fn(i64, i64) -> bool,
	mut test: impl FnMut(i64, i64) -> Option<T>,
) -> Option<T> {
	let mut found = None;
	// The original's callback skips tiles beyond `dist`; checking that before `in_grid`
	// and the marks is the same, as neither has side effects.
	let enter = |q, r| hex_distance(q, r, center.0, center.1) <= dist && in_grid(q, r);
	let visit = |q, r| match test(q, r) {
		Some(hit) => {
			found = Some(hit);
			Visit::Stop
		}
		None => Visit::Expand,
	};
	// Only tiles within `dist` are ever entered, and those lie within `dist` on both
	// axial axes, so the window covers every mark.
	if dist <= WINDOW_MAX_DIST {
		flood(center, &mut Window::new(center, dist.max(0)), enter, visit);
	} else {
		flood(center, &mut std::collections::HashSet::new(), enter, visit);
	}
	found
}

/// `World.findNextTileOfType`'s candidates, in the order it prefers them: only tiles
/// whose axial offset from the centre is even in both components (the centre first),
/// nearest ring first, each ring walked clockwise from its N corner.
pub(crate) fn even_lattice_rings(center: (i64, i64), dist: i64) -> Vec<(i64, i64)> {
	// Ring walk in lattice units (one unit = 2 tiles): start at k*N, then k steps each
	// of SE, S, SW, NW, N, NE.
	const WALK: [(i64, i64); 6] = [(1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1), (1, 0)];
	let mut out = vec![center];
	let mut k = 1;
	while 2 * k <= dist {
		let (mut a, mut b) = (0, k);
		for (da, db) in WALK {
			for _ in 0..k {
				out.push((center.0 + 2 * a, center.1 + 2 * b));
				a += da;
				b += db;
			}
		}
		k += 1;
	}
	out
}

#[cfg(test)]
mod tests {
	use super::{even_lattice_rings, flood_last};

	fn ax(x: i64, y: i64) -> (i64, i64) {
		(x, y - x / 2)
	}

	/// The original's answers when every tile within 3 of square (60, 60) holds a
	/// region and each answer is cleared before the next query.
	#[test]
	fn flood_answers_like_the_original() {
		let want = "59,59 58,59 57,58 57,59 57,60 58,61 59,61 60,62 61,61 62,61 63,60 63,59 62,59 61,58 60,58 59,57 60,57 61,57 62,58 63,58 63,61 62,60 62,62 61,62 60,63 59,62 58,62 57,61 58,58 58,60 59,58 61,59 59,60 61,60 60,59 60,61";
		let center = ax(60, 60);
		let mut has_region: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
		for q in 50..71 {
			for r in 20..41 {
				let (dq, dr) = (q - center.0, r - center.1);
				if (dq.abs() + dr.abs() + (dq + dr).abs()) / 2 <= 3 {
					has_region.insert((q, r));
				}
			}
		}
		let mut got = Vec::new();
		while let Some((q, r)) = flood_last(center, 3, |_, _| true, |q, r| has_region.contains(&(q, r)).then_some((q, r))) {
			got.push(format!("{},{}", q, r + q / 2));
			has_region.remove(&(q, r));
		}
		assert_eq!(got.join(" "), want);
	}

	#[test]
	fn lattice_rings_match_the_original_preference() {
		let got: Vec<String> = even_lattice_rings(ax(60, 60), 4)
			.into_iter()
			.map(|(q, r)| format!("{},{}", q, r + q / 2))
			.collect();
		assert_eq!(
			got.join(" "),
			"60,60 60,62 62,61 62,59 60,58 58,59 58,61 60,64 62,63 64,62 64,60 64,58 62,57 60,56 58,57 56,58 56,60 56,62 58,63"
		);
	}
}
