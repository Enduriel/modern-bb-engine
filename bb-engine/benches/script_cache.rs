//! Loading every vanilla script through `DatScriptSource`, as the module system does:
//! without the script cache (decrypt + transcode each `.cnut`), and from a warm cache
//! in a temporary directory. Skipped (with a note) when the Steam install is missing.
//!
//! `cargo bench -p bb-engine --bench script_cache`.

use std::hint::black_box;
use std::path::Path;
use std::time::Duration;

use bb_engine::harness::{STEAM_DATA_DIR, data_archives};
use bb_engine::{DatScriptSource, ScriptCache};
use criterion::{Criterion, criterion_group, criterion_main};
use squirrel_bb::ScriptSource;

fn bench(c: &mut Criterion) {
	let Ok(paths) = data_archives(Path::new(STEAM_DATA_DIR)) else {
		eprintln!("no Battle Brothers install at {STEAM_DATA_DIR}; skipping");
		return;
	};
	let root = std::env::temp_dir().join(format!("bb-script-cache-bench-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);
	let uncached = DatScriptSource::open(&paths).unwrap();
	let cached = DatScriptSource::open(&paths)
		.unwrap()
		.with_cache(Some(ScriptCache::at(&root)));
	let scripts = uncached.list_prefix("scripts/").unwrap();
	for s in &scripts {
		cached.load(s); // warm the cache
	}

	let mut group = c.benchmark_group("script_cache");
	group
		.sample_size(20)
		.measurement_time(Duration::from_secs(10));
	group.bench_function("load_all_uncached", |b| {
		b.iter(|| {
			scripts
				.iter()
				.for_each(|s| drop(black_box(uncached.load(s))))
		})
	});
	group.bench_function("load_all_warm_cache", |b| {
		b.iter(|| scripts.iter().for_each(|s| drop(black_box(cached.load(s)))))
	});
	group.finish();
	let _ = std::fs::remove_dir_all(&root);
}

criterion_group!(benches, bench);
criterion_main!(benches);
