//! The world-map pathfinder on the recorded worldgen workload (see
//! `support/nav_fixture.rs`): the whole replay, and the slowest and the median single
//! search.
//!
//! Compare against a saved baseline:
//! `cargo bench -p squirrel-bb --bench world_nav -- --save-baseline before`, change the
//! code, then `cargo bench -p squirrel-bb --bench world_nav -- --baseline before`. On a
//! busy machine, run-to-run noise can reach ~10%.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};

#[path = "support/nav_fixture.rs"]
mod nav_fixture;
#[allow(dead_code)]
#[path = "../src/native/world_nav.rs"]
mod world_nav;

use nav_fixture::NavFixture;

const FIXTURE: &str = concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/benches/data/nav_rust-engine-test.txt"
);

fn bench(c: &mut Criterion) {
	let fx = NavFixture::load(FIXTURE);

	// Rank the searches by time (best of three, each on its own grid).
	let mut ranked: Vec<(Duration, usize)> = (0..fx.len())
		.map(|i| {
			let (grid, q) = (fx.grid_at(i), fx.query(i));
			let best = (0..3)
				.map(|_| {
					let t = Instant::now();
					black_box(grid.find(q, &fx.settings[q.settings]));
					t.elapsed()
				})
				.min()
				.unwrap();
			(best, i)
		})
		.collect();
	ranked.sort();

	let mut group = c.benchmark_group("find_path");
	group
		.sample_size(20)
		.measurement_time(Duration::from_secs(15));
	group.bench_function("worldgen_replay", |b| {
		b.iter(|| fx.replay(|_, route| drop(black_box(route))))
	});
	for (name, rank) in [
		("slowest_search", ranked.len() - 1),
		("median_search", ranked.len() / 2),
	] {
		let i = ranked[rank].1;
		let (grid, q) = (fx.grid_at(i), fx.query(i));
		let settings = &fx.settings[q.settings];
		group.bench_function(name, |b| b.iter(|| black_box(grid.find(q, settings))));
	}
	group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
