//! Differential fuzz: the optimized pathfinder against the reference implementation
//! (`support/world_nav_reference.rs`) on random grids, settings and endpoints, including
//! adversarial costs and every option the recorded workload never uses. Grids honour
//! `find_path`'s contract (one id per in-grid coordinate, fixed tile data), with dense,
//! permuted and sparse ids. `NAV_FUZZ_CASES` (default 3000 grids, 4 queries each) and
//! `NAV_FUZZ_SEED` widen a run.

#[allow(dead_code)]
#[path = "../src/native/world_nav.rs"]
mod world_nav;
#[allow(dead_code, clippy::manual_is_multiple_of)] // verbatim reference
#[path = "support/world_nav_reference.rs"]
mod world_nav_reference;

/// splitmix64.
struct Rng(u64);

impl Rng {
	fn next(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
		let mut z = self.0;
		z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
		z ^ (z >> 31)
	}

	fn below(&mut self, n: u64) -> u64 {
		self.next() % n
	}

	fn range(&mut self, lo: i64, hi: i64) -> i64 {
		lo + self.below((hi - lo + 1) as u64) as i64
	}

	fn chance(&mut self, num: u64, den: u64) -> bool {
		self.below(den) < num
	}

	fn pick<T: Copy>(&mut self, v: &[T]) -> T {
		v[self.below(v.len() as u64) as usize]
	}
}

#[derive(Clone, Copy, Debug)]
struct Tile {
	id: usize,
	type_: usize,
	road: bool,
	heat: u8,
}

#[derive(Debug)]
struct Grid {
	w: i64,
	h: i64,
	cells: Vec<Option<Tile>>,
}

impl Grid {
	fn at(&self, q: i64, r: i64) -> Option<Tile> {
		let (col, row) = (q, r + q / 2);
		if col < 0 || col >= self.w || row < 0 || row >= self.h {
			return None;
		}
		self.cells[(row * self.w + col) as usize]
	}
}

#[derive(Debug)]
struct Settings {
	costs: [i32; 32],
	road_mult: f32,
	heat_cost: i32,
	road_only: bool,
	stop_at_road: bool,
}

fn random_grid(rng: &mut Rng) -> Grid {
	let (w, h) = (rng.range(1, 48), rng.range(1, 48));
	let n = (w * h) as usize;
	// Unique ids: dense, a permutation, or sparse with an offset and stride.
	let mut ids: Vec<usize> = (0..n).collect();
	match rng.below(3) {
		0 => {}
		1 => {
			for i in (1..n).rev() {
				ids.swap(i, rng.below(i as u64 + 1) as usize);
			}
		}
		_ => {
			let (offset, stride) = (rng.below(100_000) as usize, 1 + rng.below(36) as usize);
			ids.iter_mut().for_each(|id| *id = offset + *id * stride);
		}
	}
	let holes = rng.below(31);
	let roads = rng.below(91);
	let types = rng.range(2, 40) as u64;
	let heat_max = rng.pick(&[0u64, 3, 255]);
	let cells = ids
		.into_iter()
		.map(|id| {
			(rng.below(100) >= holes).then(|| Tile {
				id,
				type_: if rng.chance(1, 500) {
					usize::MAX
				} else {
					rng.below(types) as usize
				},
				road: rng.below(100) < roads,
				heat: rng.below(heat_max + 1) as u8,
			})
		})
		.collect();
	Grid { w, h, cells }
}

fn random_cost(rng: &mut Rng, mode: u64) -> i32 {
	match mode {
		// Realistic (truncated vanilla speeds).
		0 => rng.pick(&[0, 1, 1, 1, 2, 2, 2, 2, 3, 4, 15, 20, 11, 13, 10]),
		// All equal: maximal ties.
		1 => 2,
		_ => match rng.below(12) {
			0 => 0,
			1 => i32::MIN,
			2 => i32::MAX,
			3 => 214_748_364,
			4 => 214_748_365,
			5 => -(rng.range(1, 50) as i32),
			6 => rng.next() as i32,
			7 => -214_748_365,
			_ => rng.range(1, 30) as i32,
		},
	}
}

fn random_settings(rng: &mut Rng) -> Settings {
	let mode = rng.below(3);
	Settings {
		costs: std::array::from_fn(|_| random_cost(rng, mode)),
		road_mult: rng.pick(&[
			0.0,
			-0.0,
			0.25,
			0.5,
			1.0,
			2.0,
			-1.0,
			f32::NAN,
			f32::INFINITY,
			1e-30,
			0.0333,
		]) * if rng.chance(1, 10) { -1.0 } else { 1.0 },
		heat_cost: rng.pick(&[0, 0, 1, 3, 100, -7, i32::MIN, i32::MAX]),
		road_only: rng.chance(1, 4),
		stop_at_road: rng.chance(1, 4),
	}
}

fn random_point(rng: &mut Rng, g: &Grid) -> (i64, i64) {
	let col = rng.range(-2, g.w + 1);
	let row = rng.range(-2, g.h + 1);
	(col, row - col / 2)
}

fn optimized(
	g: &Grid,
	s: &Settings,
	start: (i64, i64),
	goal: (i64, i64),
	max_len: i64,
) -> Vec<usize> {
	let settings = world_nav::NavSettings {
		costs: s.costs,
		road_mult: s.road_mult,
		heat_cost: s.heat_cost,
		road_only: s.road_only,
		stop_at_road: s.stop_at_road,
	};
	world_nav::find_path(start, goal, max_len, &settings, |q, r| {
		g.at(q, r).map(|t| world_nav::NavTile {
			id: t.id,
			type_: t.type_,
			road: t.road,
			heat: t.heat,
		})
	})
}

fn reference(
	g: &Grid,
	s: &Settings,
	start: (i64, i64),
	goal: (i64, i64),
	max_len: i64,
) -> Vec<usize> {
	let settings = world_nav_reference::NavSettings {
		costs: s.costs,
		road_mult: s.road_mult,
		heat_cost: s.heat_cost,
		road_only: s.road_only,
		stop_at_road: s.stop_at_road,
	};
	world_nav_reference::find_path(start, goal, max_len, &settings, |q, r| {
		g.at(q, r).map(|t| world_nav_reference::NavTile {
			id: t.id,
			type_: t.type_,
			road: t.road,
			heat: t.heat,
		})
	})
}

#[test]
fn optimized_matches_reference() {
	let env = |k: &str, d: u64| std::env::var(k).ok().map_or(d, |v| v.parse().unwrap());
	let mut rng = Rng(env("NAV_FUZZ_SEED", 1));
	for case in 0..env("NAV_FUZZ_CASES", 3000) {
		let g = random_grid(&mut rng);
		let s = random_settings(&mut rng);
		for _ in 0..4 {
			let start = random_point(&mut rng, &g);
			let goal = match rng.below(8) {
				0 => start,
				1 => (start.0, start.1 + 1),
				_ => random_point(&mut rng, &g),
			};
			let (small, large) = (rng.range(3, 10), rng.range(10, 100));
			let max_len = rng.pick(&[0, 0, 0, 1, 2, small, large, -3, i64::MAX, i64::MIN]);
			let (a, b) = (
				optimized(&g, &s, start, goal, max_len),
				reference(&g, &s, start, goal, max_len),
			);
			assert_eq!(
				a, b,
				"case {case}: start {start:?} goal {goal:?} max_len {max_len}\n{s:?}\n{g:?}"
			);
		}
	}
}
