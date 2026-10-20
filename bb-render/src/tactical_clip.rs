//! The ground in front of a unit hides what of it is below the ground: a unit rising from
//! the ground (`actor.riseFromGround`, its `setPos(0, -200 → 0)`) comes up out of its
//! tile, its lower part hidden behind the tiles in front of it (the original's
//! screenshots, 2026-10-20: the cut follows the upper edges of the hexes south, south-west
//! and south-east of the unit's tile, and takes the unit's bars with it). Vanilla only
//! raises a unit so when the tile south of it is discovered and not lower.
//!
//! Each of those three neighbours that is drawn (discovered) and not lower than the
//! unit's tile hides what is below its upper edge. Hex geometry (tiles 160 × 80, columns
//! 120 apart, y up, from the unit's tile's anchor): the south hex's upper edge is flat at
//! −40 for |dx| ≤ 40 and falls to −80 at |dx| = 80; the south-west hex's runs from
//! (−40, −40) up to (−80, 0), then flat at 0 to −160 (south-east mirrored). A unit at
//! rest stands above all of them (its socket ends 36 units below its anchor), so only
//! units sunk into the ground are cut.

use crate::sprite::Quad;

/// Which of a tile's front neighbours hide what is below their upper edges, and where
/// the tile's anchor is.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrontEdges {
	pub anchor: [f32; 2],
	pub south_west: bool,
	pub south: bool,
	pub south_east: bool,
}

/// `y ≥ a·dx + b` (dx from the anchor, y relative to it).
#[derive(Clone, Copy)]
struct Above {
	a: f32,
	b: f32,
}

impl FrontEdges {
	fn any(&self) -> bool {
		self.south_west || self.south || self.south_east
	}

	/// The vertical strips (dx ranges) in which the hidden part's boundary is the upper
	/// envelope of straight lines, and those lines.
	fn strips(&self) -> Vec<(f32, f32, Vec<Above>)> {
		let mut strips = Vec::new();
		let line = |on: bool, a: f32, b: f32| on.then_some(Above { a, b });
		let (sw, s, se) = (self.south_west, self.south, self.south_east);
		strips.push((-160.0, -80.0, line(sw, 0.0, 0.0).into_iter().collect()));
		strips.push((-80.0, -40.0, [line(sw, -1.0, -80.0), line(s, 1.0, 0.0)].into_iter().flatten().collect()));
		strips.push((-40.0, 40.0, line(s, 0.0, -40.0).into_iter().collect()));
		strips.push((40.0, 80.0, [line(se, 1.0, -80.0), line(s, -1.0, 0.0)].into_iter().flatten().collect()));
		strips.push((80.0, 160.0, line(se, 0.0, 0.0).into_iter().collect()));
		strips
	}

	/// The highest the boundary gets over `[dx0, dx1]` (None: nothing is hidden there).
	fn highest(&self, dx0: f32, dx1: f32) -> Option<f32> {
		let mut top: Option<f32> = None;
		for (lo, hi, lines) in self.strips() {
			let (l, h) = (dx0.max(lo), dx1.min(hi));
			if l > h {
				continue;
			}
			for line in lines {
				let y = (line.a * l + line.b).max(line.a * h + line.b);
				top = Some(top.map_or(y, |t: f32| t.max(y)));
			}
		}
		top
	}
}

/// Cut the quads from `from` on (a unit's sprites and bars) where the tiles in front
/// hide them.
pub(crate) fn clip_quads(quads: &mut Vec<Quad>, from: usize, edges: &FrontEdges) {
	if !edges.any() || from >= quads.len() {
		return;
	}
	let [ax, ay] = edges.anchor;
	let drawn: Vec<Quad> = quads.drain(from..).collect();
	for quad in drawn {
		let xs = quad.corners.map(|c| c[0] - ax);
		let ys = quad.corners.map(|c| c[1] - ay);
		let (x0, x1) = (xs.iter().copied().fold(f32::MAX, f32::min), xs.iter().copied().fold(f32::MIN, f32::max));
		let y0 = ys.iter().copied().fold(f32::MAX, f32::min);
		// Wholly above what can hide it: as it is.
		if edges.highest(x0, x1).is_none_or(|top| y0 >= top) {
			quads.push(quad);
			continue;
		}
		let corners: Vec<[f32; 2]> = (0..4).map(|i| [xs[i], ys[i]]).collect();
		// The parts left of, between and right of the strips aren't hidden.
		let mut pieces: Vec<(f32, f32, Vec<Above>)> = vec![(f32::NEG_INFINITY, -160.0, Vec::new())];
		pieces.extend(edges.strips());
		pieces.push((160.0, f32::INFINITY, Vec::new()));
		for (lo, hi, lines) in pieces {
			if hi <= x0 || lo >= x1 {
				continue;
			}
			let mut poly = corners.clone();
			if lo > x0 {
				poly = clip(&poly, |p| p[0] - lo);
			}
			if hi < x1 {
				poly = clip(&poly, |p| hi - p[0]);
			}
			for line in &lines {
				poly = clip(&poly, |p| p[1] - (line.a * p[0] + line.b));
			}
			emit(quads, &quad, &poly, [ax, ay]);
		}
	}
}

/// Sutherland–Hodgman: the part of convex `poly` where `inside(p) ≥ 0`.
fn clip(poly: &[[f32; 2]], inside: impl Fn([f32; 2]) -> f32) -> Vec<[f32; 2]> {
	let mut out = Vec::with_capacity(poly.len() + 1);
	for i in 0..poly.len() {
		let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
		let (dp, dq) = (inside(p), inside(q));
		if dp >= 0.0 {
			out.push(p);
		}
		if (dp >= 0.0) != (dq >= 0.0) {
			let t = dp / (dp - dq);
			out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
		}
	}
	out
}

/// `poly` (relative to `anchor`) drawn as `quad` draws it: its texture coordinates and
/// colours where the quad has them at those points, as a fan of quads.
fn emit(quads: &mut Vec<Quad>, quad: &Quad, poly: &[[f32; 2]], anchor: [f32; 2]) {
	if poly.len() < 3 {
		return;
	}
	// The quad is a parallelogram: corner 0 plus s × (1 − 0) plus t × (3 − 0).
	let c = quad.corners;
	let (e1, e2) = ([c[1][0] - c[0][0], c[1][1] - c[0][1]], [c[3][0] - c[0][0], c[3][1] - c[0][1]]);
	let det = e1[0] * e2[1] - e1[1] * e2[0];
	if det == 0.0 {
		return;
	}
	let at = |p: [f32; 2]| {
		let (x, y) = (p[0] + anchor[0] - c[0][0], p[1] + anchor[1] - c[0][1]);
		((x * e2[1] - y * e2[0]) / det, (e1[0] * y - e1[1] * x) / det)
	};
	let uv = |(s, t): (f32, f32)| {
		let u = quad.uv;
		let d1 = [u[1][0] - u[0][0], u[1][1] - u[0][1]];
		let d2 = [u[3][0] - u[0][0], u[3][1] - u[0][1]];
		[u[0][0] + d1[0] * s + d2[0] * t, u[0][1] + d1[1] * s + d2[1] * t]
	};
	let color = |(s, t): (f32, f32)| {
		quad.corner_colors.map(|cc| {
			let mix = |a: [u8; 4], b: [u8; 4], t: f32| -> [f32; 4] {
				std::array::from_fn(|i| f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t)
			};
			let (top, bottom) = (mix(cc[0], cc[1], s), mix(cc[3], cc[2], s));
			std::array::from_fn::<u8, 4, _>(|i| (top[i] + (bottom[i] - top[i]) * t).round() as u8)
		})
	};
	let vertex = |p: [f32; 2]| ([p[0] + anchor[0], p[1] + anchor[1]], uv(at(p)), color(at(p)));
	let v0 = vertex(poly[0]);
	let mut i = 1;
	while i + 1 < poly.len() {
		let (a, b) = (vertex(poly[i]), vertex(poly[i + 1]));
		let d = if i + 2 < poly.len() { vertex(poly[i + 2]) } else { b };
		let corners = [v0, a, b, d];
		quads.push(Quad {
			corners: corners.map(|v| v.0),
			uv: corners.map(|v| v.1),
			corner_colors: quad.corner_colors.map(|_| corners.map(|v| v.2.unwrap_or(quad.color))),
			..*quad
		});
		i += 2;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn quad(left: f32, right: f32, bottom: f32, top: f32) -> Quad {
		Quad {
			texture: 0,
			corners: [[left, top], [right, top], [right, bottom], [left, bottom]],
			uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
			color: [255; 4],
			corner_colors: None,
			overlay: [0; 4],
			factor: 0.0,
			saturation: 1.0,
			flags: 0,
			level: 0,
		}
	}

	const ALL: FrontEdges = FrontEdges {
		anchor: [1000.0, 500.0],
		south_west: true,
		south: true,
		south_east: true,
	};

	fn area(quads: &[Quad]) -> f32 {
		quads
			.iter()
			.map(|q| {
				let c = q.corners;
				let tri = |a: [f32; 2], b: [f32; 2], d: [f32; 2]| ((b[0] - a[0]) * (d[1] - a[1]) - (d[0] - a[0]) * (b[1] - a[1])).abs() / 2.0;
				tri(c[0], c[1], c[2]) + tri(c[0], c[2], c[3])
			})
			.sum()
	}

	#[test]
	fn a_unit_at_rest_is_left_alone() {
		let mut quads = vec![quad(970.0, 1030.0, 470.0, 600.0)];
		clip_quads(&mut quads, 0, &ALL);
		assert_eq!(quads.len(), 1);
		assert_eq!(quads[0].corners, quad(970.0, 1030.0, 470.0, 600.0).corners);
	}

	#[test]
	fn a_sunk_unit_shows_only_above_the_hexes_in_front() {
		// 100 wide (dx −50..50), from 100 below the anchor to 50 above.
		let mut quads = vec![quad(950.0, 1050.0, 400.0, 550.0)];
		clip_quads(&mut quads, 0, &ALL);
		// Kept: above −40 for |dx| ≤ 40, above the slopes (−40 → −30) for 40 < |dx| ≤ 50.
		let kept = 80.0 * 90.0 + 2.0 * 10.0 * (90.0 - 5.0);
		assert!((area(&quads) - kept).abs() < 0.01, "{} vs {kept}", area(&quads));
		for q in &quads {
			for (p, uv) in q.corners.iter().zip(q.uv) {
				assert!(p[1] - 500.0 >= -40.0 - 1e-3, "{p:?}");
				// The texture stays where it was: u along x, v along y.
				assert!((uv[0] - (p[0] - 950.0) / 100.0).abs() < 1e-5 && (uv[1] - (p[1] - 400.0) / 150.0).abs() < 1e-5);
			}
		}
	}

	#[test]
	fn an_undiscovered_or_lower_neighbour_hides_nothing() {
		let only_south = FrontEdges {
			south_west: false,
			south_east: false,
			..ALL
		};
		let mut quads = vec![quad(900.0, 1100.0, 400.0, 550.0)];
		clip_quads(&mut quads, 0, &only_south);
		// The south hex hides below −40 (|dx| ≤ 40) and below its slopes down to −80
		// (|dx| 40..80); nothing past |dx| = 80.
		let hidden = 80.0 * 60.0 + 2.0 * (40.0 * 40.0 / 2.0 + 40.0 * 20.0);
		assert!((area(&quads) - (200.0 * 150.0 - hidden)).abs() < 0.01, "{}", area(&quads));
	}
}
