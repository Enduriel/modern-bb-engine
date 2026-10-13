//! Archive access on the real install: opening every `data_*.dat`, and opening them and
//! reading every script entry. Skipped (with a note) when the Steam install is missing.
//!
//! `cargo bench -p bb-engine --bench archives -- --save-baseline before`, change the
//! code, then `-- --baseline before`.

use std::hint::black_box;
use std::path::Path;
use std::time::Duration;

use bb_engine::DatArchive;
use bb_engine::harness::{STEAM_DATA_DIR, data_archives};
use criterion::{Criterion, criterion_group, criterion_main};

fn bench(c: &mut Criterion) {
	let Ok(paths) = data_archives(Path::new(STEAM_DATA_DIR)) else {
		eprintln!("no Battle Brothers install at {STEAM_DATA_DIR}; skipping");
		return;
	};
	let mut group = c.benchmark_group("archives");
	group.sample_size(20).measurement_time(Duration::from_secs(10));
	group.bench_function("open_all", |b| {
		b.iter(|| {
			for p in &paths {
				black_box(DatArchive::open(p).unwrap());
			}
		})
	});
	group.bench_function("open_and_read_all_scripts", |b| {
		b.iter(|| {
			for p in &paths {
				let mut archive = DatArchive::open(p).unwrap();
				for name in archive.names() {
					if name.starts_with("scripts/") {
						black_box(archive.read(&name));
					}
				}
			}
		})
	});
	group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
