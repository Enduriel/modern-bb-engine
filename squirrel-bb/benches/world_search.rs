//! `World.findNextRegion`'s flood on the recorded worldgen workload (see
//! `support/region_fixture.rs`): the whole replay, and the slowest and the median call.
//!
//! Compare against a saved baseline:
//! `cargo bench -p squirrel-bb --bench world_search -- --save-baseline before`, change the
//! code, then `cargo bench -p squirrel-bb --bench world_search -- --baseline before`. On a
//! busy machine, run-to-run noise can reach ~10%.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};

#[path = "support/region_fixture.rs"]
mod region_fixture;
#[allow(dead_code, unused_imports)] // its unit tests come along
#[path = "../src/native/world_search.rs"]
mod world_search;

use region_fixture::RegionFixture;

const FIXTURE: &str = concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/benches/data/regions_rust-engine-test.txt"
);

fn bench(c: &mut Criterion) {
	let fx = RegionFixture::load(FIXTURE);

	// Rank the calls by time (best of three), then keep the grids of the two benched.
	let mut ranked = Vec::new();
	fx.replay(|grid, call| {
		let best = (0..3)
			.map(|_| {
				let t = Instant::now();
				black_box(grid.find_next_region(call.col, call.row, call.dist, call.type_));
				t.elapsed()
			})
			.min()
			.unwrap();
		ranked.push((best, ranked.len()));
	});
	ranked.sort();
	let picks = [
		("slowest_call", ranked[ranked.len() - 1].1),
		("median_call", ranked[ranked.len() / 2].1),
	];
	let mut picked = Vec::new();
	let mut i = 0;
	fx.replay(|grid, call| {
		for (name, at) in picks {
			if at == i {
				picked.push((
					name,
					grid.clone(),
					(call.col, call.row, call.dist, call.type_),
				));
			}
		}
		i += 1;
	});

	let mut group = c.benchmark_group("find_next_region");
	group
		.sample_size(20)
		.measurement_time(Duration::from_secs(10));
	group.bench_function("worldgen_replay", |b| {
		b.iter(|| {
			fx.replay(|grid, call| {
				black_box(grid.find_next_region(call.col, call.row, call.dist, call.type_));
			})
		})
	});
	for (name, grid, (col, row, dist, type_)) in &picked {
		group.bench_function(*name, |b| {
			b.iter(|| black_box(grid.find_next_region(*col, *row, *dist, *type_)))
		});
	}
	group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
