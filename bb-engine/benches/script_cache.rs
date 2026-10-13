//! The script cache, in a temporary directory:
//! - every vanilla script loaded through `DatScriptSource` without the cache (decrypt
//!   and transcode each `.cnut`) and from a warm cache (skipped without the Steam
//!   install);
//! - every mod `.nut` in the zips under `BB_BENCH_MODS=<dir>` read and compiled, against
//!   loaded from a warm compile cache, as the module system does (skipped if unset).
//!
//! `cargo bench -p bb-engine --bench script_cache`.

use std::hint::black_box;
use std::path::Path;
use std::time::Duration;

use bb_engine::harness::{STEAM_DATA_DIR, data_archives};
use bb_engine::{DatScriptSource, ScriptCache};
use criterion::{Criterion, criterion_group, criterion_main};
use squirrel::SQVM;
use squirrel_bb::{LoadedScript, ScriptSource};

/// Load `path` the way the module system does for a source with a compile cache: from
/// the cache if possible, otherwise compile and offer the result to the cache.
fn load_compiled(vm: &SQVM, source: &DatScriptSource, path: &str) {
	let context = || 0x5eed;
	match source.load_for(path, &context) {
		Some(LoadedScript::Bytecode(bytes)) => drop(black_box(vm.load_closure(&bytes))),
		Some(LoadedScript::Source(text)) => {
			let closure = vm.compile(&String::from_utf8_lossy(&text), path);
			if let Ok(closure) = &closure
				&& source.caches_compiles()
				&& let Ok(bytes) = closure.write_closure(vm)
			{
				source.compiled(path, context(), &bytes);
			}
			drop(black_box(closure));
		}
		None => {}
	}
}

fn bench_mods(c: &mut Criterion) {
	let Some(dir) = std::env::var_os("BB_BENCH_MODS") else {
		eprintln!("BB_BENCH_MODS not set; skipping the mod compile benchmark");
		return;
	};
	let mut zips: Vec<_> = std::fs::read_dir(&dir)
		.unwrap()
		.flatten()
		.map(|e| e.path())
		.collect();
	zips.sort();
	let root = std::env::temp_dir().join(format!("bb-compile-cache-bench-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);
	let uncached = DatScriptSource::open(&zips).unwrap();
	let cached = DatScriptSource::open(&zips)
		.unwrap()
		.with_cache(Some(ScriptCache::at(&root)));
	let scripts = uncached.list_prefix("").unwrap_or_default();
	let vm = SQVM::new(1024);
	for s in &scripts {
		load_compiled(&vm, &cached, s); // warm the cache
	}

	let mut group = c.benchmark_group("compile_cache");
	group
		.sample_size(10)
		.measurement_time(Duration::from_secs(10));
	group.bench_function("mods_compile_uncached", |b| {
		b.iter(|| {
			scripts
				.iter()
				.for_each(|s| load_compiled(&vm, &uncached, s))
		})
	});
	group.bench_function("mods_warm_cache", |b| {
		b.iter(|| scripts.iter().for_each(|s| load_compiled(&vm, &cached, s)))
	});
	group.finish();
	let _ = std::fs::remove_dir_all(&root);
}

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

criterion_group!(benches, bench, bench_mods);
criterion_main!(benches);
