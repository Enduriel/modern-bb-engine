//! Differential fuzz: the optimized flood (`World.findNextRegion`) against the reference
//! implementation (`support/world_search_reference.rs`) on random grids, region layouts,
//! centres (including off-grid) and distances (including negative ones and ones large
//! enough to take the hash-set path). Compares the whole sequence of tiles offered to the
//! match test, not just the answer. `NAV_FUZZ_CASES` (default 3000) and `NAV_FUZZ_SEED`
//! widen a run.

#[allow(dead_code)]
#[path = "../src/native/world_search.rs"]
mod world_search;
#[allow(dead_code)]
#[path = "support/world_search_reference.rs"]
mod world_search_reference;

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
}

struct Grid {
	w: i64,
	h: i64,
	/// `(type, region)` per tile, square order.
	tiles: Vec<(i64, i64)>,
}

impl Grid {
	fn idx(&self, q: i64, r: i64) -> Option<usize> {
		let (col, row) = (q, r + q / 2);
		(col >= 0 && col < self.w && row >= 0 && row < self.h)
			.then(|| (row * self.w + col) as usize)
	}
}

/// The answer and every tile offered to the test, in order.
type Trace = (Option<i64>, Vec<(i64, i64)>);

fn optimized(g: &Grid, center: (i64, i64), dist: i64, ty: i64) -> Trace {
	let mut offered = Vec::new();
	let found = world_search::flood_last(
		center,
		dist,
		|q, r| g.idx(q, r).is_some(),
		|q, r| {
			offered.push((q, r));
			let (t, region) = g.tiles[g.idx(q, r)?];
			(region != 0 && (ty == 0 || t == ty)).then_some(region)
		},
	);
	(found, offered)
}

fn reference(g: &Grid, center: (i64, i64), dist: i64, ty: i64) -> Trace {
	let mut offered = Vec::new();
	let found = world_search_reference::flood_last(
		center,
		dist,
		|q, r| g.idx(q, r).is_some(),
		|q, r| {
			offered.push((q, r));
			let (t, region) = g.tiles[g.idx(q, r)?];
			(region != 0 && (ty == 0 || t == ty)).then_some(region)
		},
	);
	(found, offered)
}

#[test]
fn optimized_matches_reference() {
	let env = |k: &str, d: u64| std::env::var(k).ok().map_or(d, |v| v.parse().unwrap());
	let mut rng = Rng(env("NAV_FUZZ_SEED", 1));
	for case in 0..env("NAV_FUZZ_CASES", 3000) {
		let (w, h) = (rng.range(1, 60), rng.range(1, 60));
		let (types, density) = (rng.range(1, 6), rng.below(101));
		let tiles = (0..w * h)
			.map(|_| {
				let region = if rng.below(100) < density {
					rng.range(1, 50)
				} else {
					0
				};
				(rng.range(1, types), region)
			})
			.collect();
		let g = Grid { w, h, tiles };
		for _ in 0..4 {
			let col = rng.range(-3, w + 2);
			let center = (col, rng.range(-3, h + 2) - col / 2);
			let dist = match rng.below(10) {
				0 => rng.range(-3, 0),
				1 => rng.range(128, 200),
				_ => rng.range(1, 20),
			};
			let ty = if rng.below(3) == 0 {
				0
			} else {
				rng.range(1, types)
			};
			let (a, b) = (
				optimized(&g, center, dist, ty),
				reference(&g, center, dist, ty),
			);
			assert!(
				a == b,
				"case {case}: center {center:?} dist {dist} type {ty} on {w}x{h}: {:?} vs {:?}",
				a.0,
				b.0
			);
		}
	}
}
