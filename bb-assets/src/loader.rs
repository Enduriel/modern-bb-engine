//! On-demand image loading: images decode on worker threads when first asked for,
//! and their owner (the renderer) takes them over as they finish.
//!
//! This replaces the original's preload lists. Vanilla reads
//! `preload/on_start.txt` and `preload/on_running.txt` and loads every listed atlas
//! and sound up front. Those files can't be merged between mods (each replaces the
//! whole list), so we never read them. Instead an image starts loading when something
//! first needs it, or earlier when a script hints at it (`Tactical.addResource`).
//!
//! [`ImageLoader`] is the decode service: a queue, worker threads and a result
//! channel. [`TextureCache`] sits on top and remembers each path's state; it is
//! generic over the uploaded texture type, so it works without a GPU.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::image::{Image, decode_png};

/// Where the loader reads files from: the game's file system (archives and mods).
pub trait FileSource: Send + Sync {
	/// The bytes of the file at `path`, or `None` if no archive has it.
	fn read(&self, path: &str) -> Option<Vec<u8>>;
}

/// A finished decode: the path asked for and its image or why it failed.
pub type Decoded = (String, Result<Image, String>);

/// Read and decode `path` from `source`.
pub fn load_image(source: &dyn FileSource, path: &str) -> Result<Image, String> {
	let bytes = source
		.read(path)
		.ok_or_else(|| format!("{path}: no such file"))?;
	decode_png(&bytes).map_err(|e| format!("{path}: {e}"))
}

/// Paths waiting for a worker, and whether the loader is shutting down.
#[derive(Default)]
struct Queue {
	jobs: Mutex<(VecDeque<String>, bool)>,
	ready: Condvar,
}

/// Decodes images on worker threads.
pub struct ImageLoader {
	source: Arc<dyn FileSource>,
	queue: Arc<Queue>,
	results: Receiver<Decoded>,
	workers: Vec<JoinHandle<()>>,
}

impl ImageLoader {
	/// A loader reading from `source` with `threads` workers (at least one).
	pub fn new(source: Arc<dyn FileSource>, threads: usize) -> Self {
		let queue = Arc::new(Queue::default());
		let (sender, results) = channel();
		let workers = (0..threads.max(1))
			.map(|i| {
				let (source, queue, sender) = (source.clone(), queue.clone(), sender.clone());
				std::thread::Builder::new()
					.name(format!("image-loader-{i}"))
					.spawn(move || work(&*source, &queue, &sender))
					.expect("spawn image loader thread")
			})
			.collect();
		ImageLoader {
			source,
			queue,
			results,
			workers,
		}
	}

	/// Queue `path` for decoding. The caller avoids queuing a path twice.
	pub fn request(&self, path: &str) {
		self.queue
			.jobs
			.lock()
			.unwrap()
			.0
			.push_back(path.to_string());
		self.queue.ready.notify_one();
	}

	/// A finished decode, if any is waiting.
	pub fn try_take(&self) -> Option<Decoded> {
		self.results.try_recv().ok()
	}

	/// Decode `path` on this thread now. If it was still queued it is taken off the
	/// queue; if a worker already has it, that result arrives later as well.
	pub fn load_now(&self, path: &str) -> Result<Image, String> {
		self.queue
			.jobs
			.lock()
			.unwrap()
			.0
			.retain(|queued| queued != path);
		load_image(&*self.source, path)
	}
}

impl Drop for ImageLoader {
	fn drop(&mut self) {
		self.queue.jobs.lock().unwrap().1 = true;
		self.queue.ready.notify_all();
		for worker in self.workers.drain(..) {
			let _ = worker.join();
		}
	}
}

/// A worker: take paths until shutdown, decode each, send the result.
fn work(source: &dyn FileSource, queue: &Queue, results: &Sender<Decoded>) {
	loop {
		let path = {
			let mut jobs = queue.jobs.lock().unwrap();
			loop {
				if jobs.1 {
					return;
				}
				if let Some(path) = jobs.0.pop_front() {
					break path;
				}
				jobs = queue.ready.wait(jobs).unwrap();
			}
		};
		let image = load_image(source, &path);
		if results.send((path, image)).is_err() {
			return;
		}
	}
}

/// Where one path's texture stands.
#[derive(Debug, PartialEq)]
pub enum Slot<T> {
	Loading,
	Ready(T),
	/// The path failed to load (why), and holds the fallback image in its place.
	Missing(T, String),
	/// The path failed to load (why), and there's no fallback.
	Failed(String),
}

impl<T> Slot<T> {
	/// The texture to draw with: the path's own, or the fallback in its place.
	pub fn texture(&self) -> Option<&T> {
		match self {
			Slot::Ready(texture) | Slot::Missing(texture, _) => Some(texture),
			Slot::Loading | Slot::Failed(_) => None,
		}
	}
}

/// Textures by path, loaded on first use. `T` is whatever the owner makes from a
/// decoded image (a GPU texture in the renderer).
///
/// A path that fails to load takes the fallback image instead, if there is one, as
/// the original's textures do (`0x534e95`): it logs
/// `Failed to load texture from file "<path>".` and loads `error.png` (its
/// "texture missing" picture) into that texture, which keeps its name (`0x535e20`).
pub struct TextureCache<T> {
	loader: ImageLoader,
	slots: HashMap<String, Slot<T>>,
	fallback: Option<String>,
}

impl<T> TextureCache<T> {
	pub fn new(loader: ImageLoader) -> Self {
		TextureCache {
			loader,
			slots: HashMap::new(),
			fallback: None,
		}
	}

	/// Load `path` in place of a texture that fails to load (the game's `error.png`).
	pub fn with_fallback(mut self, path: &str) -> Self {
		self.fallback = Some(path.to_string());
		self
	}

	/// The slot for `path` once its load finished with `result`.
	fn settle(&self, path: &str, result: Result<Image, String>, upload: &mut impl FnMut(&str, Image) -> T) -> Slot<T> {
		let err = match result {
			Ok(image) => return Slot::Ready(upload(path, image)),
			Err(err) => err,
		};
		eprintln!("Failed to load texture from file \"{path}\".");
		eprintln!("  {err}");
		let fallback = self.fallback.as_deref().filter(|f| *f != path);
		match fallback.map(|f| self.loader.load_now(f)) {
			Some(Ok(image)) => Slot::Missing(upload(path, image), err),
			Some(Err(fallback_err)) => {
				eprintln!("Failed to load texture from file \"{}\".", fallback.unwrap_or_default());
				eprintln!("  {fallback_err}");
				Slot::Failed(err)
			}
			None => Slot::Failed(err),
		}
	}

	/// Start loading `path` in the background unless it is already known: a hint
	/// that it will be needed soon.
	pub fn prefetch(&mut self, path: &str) {
		if !self.slots.contains_key(path) {
			self.slots.insert(path.to_string(), Slot::Loading);
			self.loader.request(path);
		}
	}

	/// Take in every finished decode, turning each image into a `T` with `upload`.
	/// Call once per frame.
	pub fn pump(&mut self, mut upload: impl FnMut(&str, Image) -> T) {
		while let Some((path, result)) = self.loader.try_take() {
			if !matches!(self.slots.get(&path), Some(Slot::Loading)) {
				continue; // unknown, or already loaded on demand
			}
			let slot = self.settle(&path, result, &mut upload);
			self.slots.insert(path, slot);
		}
	}

	/// The texture for `path` (or the fallback in its place) if it is ready. The first
	/// call for a path starts loading it in the background.
	pub fn get(&mut self, path: &str) -> Option<&T> {
		self.prefetch(path);
		self.slots.get(path).and_then(Slot::texture)
	}

	/// The texture for `path` (or the fallback in its place), decoding it on this
	/// thread if it isn't ready yet.
	pub fn get_now(
		&mut self,
		path: &str,
		mut upload: impl FnMut(&str, Image) -> T,
	) -> Result<&T, &str> {
		if matches!(self.slots.get(path), None | Some(Slot::Loading)) {
			let slot = self.settle(path, self.loader.load_now(path), &mut upload);
			self.slots.insert(path.to_string(), slot);
		}
		match &self.slots[path] {
			Slot::Ready(texture) | Slot::Missing(texture, _) => Ok(texture),
			Slot::Failed(err) => Err(err),
			Slot::Loading => unreachable!("just loaded"),
		}
	}

	/// The state of `path`, if it was ever asked for.
	pub fn slot(&self, path: &str) -> Option<&Slot<T>> {
		self.slots.get(path)
	}

	/// Every path ever asked for, and its state.
	pub fn slots(&self) -> impl Iterator<Item = (&str, &Slot<T>)> {
		self.slots.iter().map(|(path, slot)| (path.as_str(), slot))
	}

	/// Forget `path`, returning its texture if it had one. Asking again reloads it.
	pub fn remove(&mut self, path: &str) -> Option<T> {
		match self.slots.remove(path)? {
			Slot::Ready(texture) | Slot::Missing(texture, _) => Some(texture),
			_ => None,
		}
	}
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::time::{Duration, Instant};

	use super::*;
	use crate::image::tests::encode;

	/// Files in memory, counting reads.
	struct Files {
		files: HashMap<String, Vec<u8>>,
		reads: AtomicUsize,
	}

	impl FileSource for Files {
		fn read(&self, path: &str) -> Option<Vec<u8>> {
			self.reads.fetch_add(1, Ordering::SeqCst);
			self.files.get(path).cloned()
		}
	}

	fn files() -> Arc<Files> {
		let mut files = HashMap::new();
		files.insert(
			"gfx/a.png".to_string(),
			encode(2, 1, png::ColorType::Rgba, &[1; 8], None),
		);
		files.insert(
			"gfx/b.png".to_string(),
			encode(1, 3, png::ColorType::Rgb, &[2; 9], None),
		);
		files.insert("gfx/broken.png".to_string(), b"nope".to_vec());
		files.insert(
			"error.png".to_string(),
			encode(3, 2, png::ColorType::Rgba, &[3; 24], None),
		);
		Arc::new(Files {
			files,
			reads: AtomicUsize::new(0),
		})
	}

	/// Pump until `path` leaves `Loading`, uploading as (width, height).
	fn pump_until(cache: &mut TextureCache<(u32, u32)>, path: &str) {
		let deadline = Instant::now() + Duration::from_secs(10);
		while matches!(cache.slot(path), Some(Slot::Loading)) {
			assert!(Instant::now() < deadline, "{path} never finished");
			cache.pump(|_, image| (image.width, image.height));
			std::thread::sleep(Duration::from_millis(1));
		}
	}

	#[test]
	fn first_use_loads_in_the_background() {
		let mut cache = TextureCache::new(ImageLoader::new(files(), 2));
		assert_eq!(cache.get("gfx/a.png"), None);
		assert_eq!(cache.slot("gfx/a.png"), Some(&Slot::Loading));
		pump_until(&mut cache, "gfx/a.png");
		assert_eq!(cache.get("gfx/a.png"), Some(&(2, 1)));
	}

	#[test]
	fn prefetching_twice_reads_the_file_once() {
		let source = files();
		let mut cache = TextureCache::new(ImageLoader::new(source.clone(), 1));
		cache.prefetch("gfx/b.png");
		cache.prefetch("gfx/b.png");
		pump_until(&mut cache, "gfx/b.png");
		assert_eq!(cache.get("gfx/b.png"), Some(&(1, 3)));
		assert_eq!(source.reads.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn get_now_loads_on_the_calling_thread() {
		let mut cache = TextureCache::new(ImageLoader::new(files(), 1));
		let texture = cache.get_now("gfx/b.png", |path, image| (path.len() as u32, image.height));
		assert_eq!(texture, Ok(&(9, 3)));
	}

	#[test]
	fn missing_and_broken_files_fail_once_and_stay_failed() {
		let mut cache = TextureCache::new(ImageLoader::new(files(), 1));
		cache.prefetch("gfx/missing.png");
		cache.prefetch("gfx/broken.png");
		pump_until(&mut cache, "gfx/missing.png");
		pump_until(&mut cache, "gfx/broken.png");
		assert_eq!(
			cache.slot("gfx/missing.png"),
			Some(&Slot::Failed("gfx/missing.png: no such file".into()))
		);
		assert!(matches!(
			cache.slot("gfx/broken.png"),
			Some(Slot::Failed(_))
		));
		assert_eq!(
			cache.get_now("gfx/missing.png", |_, i| (i.width, i.height)),
			Err("gfx/missing.png: no such file")
		);
	}

	#[test]
	fn a_texture_that_fails_takes_the_fallback_in_its_place() {
		let mut cache = TextureCache::new(ImageLoader::new(files(), 1)).with_fallback("error.png");
		cache.prefetch("gfx/missing.png");
		pump_until(&mut cache, "gfx/missing.png");
		assert_eq!(
			cache.slot("gfx/missing.png"),
			Some(&Slot::Missing((3, 2), "gfx/missing.png: no such file".into()))
		);
		assert_eq!(cache.get("gfx/missing.png"), Some(&(3, 2)), "drawn with the fallback");
		let broken = cache.get_now("gfx/broken.png", |_, i| (i.width, i.height));
		assert_eq!(broken, Ok(&(3, 2)), "on demand too");
		// Without its image, the fallback can't stand in: the failure stays one.
		let mut cache = TextureCache::new(ImageLoader::new(files(), 1)).with_fallback("gone.png");
		assert!(cache.get_now("gfx/missing.png", |_, i| (i.width, i.height)).is_err());
	}

	#[test]
	fn a_late_background_result_does_not_replace_an_on_demand_load() {
		let mut cache = TextureCache::new(ImageLoader::new(files(), 1));
		cache.prefetch("gfx/a.png");
		cache.get_now("gfx/a.png", |_, _| (100, 100)).unwrap();
		// Whatever the worker produced is dropped: the slot is no longer loading.
		std::thread::sleep(Duration::from_millis(50));
		cache.pump(|_, _| (0, 0));
		assert_eq!(cache.get("gfx/a.png"), Some(&(100, 100)));
	}

	#[test]
	fn removing_a_texture_returns_it_and_forgets_the_path() {
		let mut cache = TextureCache::new(ImageLoader::new(files(), 1));
		cache
			.get_now("gfx/a.png", |_, i| (i.width, i.height))
			.unwrap();
		assert_eq!(cache.remove("gfx/a.png"), Some((2, 1)));
		assert_eq!(cache.slot("gfx/a.png"), None);
	}

	#[test]
	fn dropping_the_loader_stops_its_workers() {
		let loader = ImageLoader::new(files(), 4);
		loader.request("gfx/a.png");
		drop(loader); // joins every worker; hangs here if one never exits
	}
}
