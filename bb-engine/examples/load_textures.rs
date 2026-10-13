//! Time loading every sprite-sheet atlas of an install: each one alone (the wait
//! on first use) and all of them on worker threads.
//!
//! ```text
//! cargo run --release -p bb-engine --example load_textures -- [data dir] [--threads N]
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_assets::{ImageLoader, Slot, TextureCache, load_image};
use bb_engine::DatArchive;
use bb_engine::assets::load_sprites;
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};

fn main() {
	let mut dir = PathBuf::from(STEAM_DATA_DIR);
	let mut threads = std::thread::available_parallelism().map_or(4, |n| n.get());
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		match arg.as_str() {
			"--threads" => threads = args.next().and_then(|n| n.parse().ok()).expect("--threads N"),
			_ => dir = PathBuf::from(arg),
		}
	}
	let paths = harness::data_archives(&dir).expect("list archives");

	let start = Instant::now();
	let mut archives: Vec<DatArchive> = paths.iter().map(|p| DatArchive::open(p).unwrap()).collect();
	let registry = load_sprites(&mut archives).registry;
	println!(
		"brushes: {} sheets, {} sprites in {:.1} ms",
		registry.sheets().len(),
		registry.len(),
		ms(start.elapsed())
	);
	let textures: Vec<&str> = registry.sheets().iter().map(|s| s.sheet.texture.as_str()).collect();
	let files = Arc::new(GameFiles::open(&paths).expect("open archives"));

	// One at a time: what the first sprite drawn from an atlas waits for.
	let mut total = Duration::ZERO;
	let mut bytes = 0;
	let mut timings = Vec::new();
	for texture in &textures {
		let start = Instant::now();
		let image = load_image(&*files, texture).expect("decode");
		let took = start.elapsed();
		total += took;
		bytes += image.rgba.len();
		timings.push((took, *texture, image.width, image.height));
	}
	timings.sort_by(|a, b| b.0.cmp(&a.0));
	println!("one at a time: {:.1} ms for {} MB of RGBA", ms(total), bytes >> 20);
	for (took, texture, width, height) in timings.iter().take(5) {
		println!("  {:7.1} ms  {texture} ({width}x{height})", ms(*took));
	}

	// All of them on worker threads, as a burst of first uses would go.
	let mut cache: TextureCache<()> = TextureCache::new(ImageLoader::new(files, threads));
	let start = Instant::now();
	for texture in &textures {
		cache.prefetch(texture);
	}
	while textures.iter().any(|t| matches!(cache.slot(t), Some(Slot::Loading))) {
		cache.pump(|_, _| ());
		std::thread::sleep(Duration::from_micros(200));
	}
	println!("all on {threads} threads: {:.1} ms", ms(start.elapsed()));
}

fn ms(d: Duration) -> f64 {
	d.as_secs_f64() * 1000.0
}
