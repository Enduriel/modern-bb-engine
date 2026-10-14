//! Frames loaded on worker threads (texture residency): a request names an
//! atlas, all its frames, and the ones wanted now. A worker reads the wanted frames'
//! regions from our cached sheet ([`crate::sheet_cache`]); without one it decodes the
//! atlas, cuts every frame's region, keeps the sheet for next time and answers from
//! that. Asking for no frame only gets the sheet ready (a warm-up).
//!
//! A sheet's index is read once and kept ([`Sheets`]), and only one worker builds a
//! missing sheet: the others wanting it wait for it, then read it like any other.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::frames::{LEVELS, Region, TexelRect, cut_levels};
use crate::mips::mip_levels;
use crate::loader::{FileSource, load_image};
use crate::sheet_cache::{Sheet, SheetCache, fnv1a};
use crate::Image;

/// Frames of one atlas to load.
#[derive(Clone, Debug)]
pub struct FrameRequest {
	pub atlas: String,
	/// Every frame of the atlas (what its sheet holds).
	pub frames: Arc<[TexelRect]>,
	/// The frames wanted now.
	pub wanted: Vec<TexelRect>,
}

/// A frame loaded: its rectangle in the atlas, its region, and the region's first mip
/// levels (`crate::frames::LEVELS`, level 0 first).
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedFrame {
	pub frame: TexelRect,
	pub region: Region,
	pub levels: Vec<Image>,
}

/// A request's answer: the atlas, and its wanted frames or why they couldn't load.
pub type LoadedFrames = (String, Result<Vec<LoadedFrame>, String>);

#[derive(Default)]
struct Queue {
	jobs: Mutex<(VecDeque<FrameRequest>, bool)>,
	ready: Condvar,
}

/// The sheets opened so far, by key, and those a worker is building.
#[derive(Default)]
struct Sheets {
	state: Mutex<(HashMap<String, Arc<Sheet>>, HashSet<String>)>,
	built: Condvar,
}

impl Sheets {
	/// The sheet for `key`: kept, or read from `cache`; or `None`, and then this caller
	/// builds it (and must call [`Sheets::done`]) unless another is already (then this
	/// waits for that one).
	fn get(&self, cache: &SheetCache, key: &str) -> Option<Arc<Sheet>> {
		let mut state = self.state.lock().unwrap();
		loop {
			if let Some(sheet) = state.0.get(key) {
				return Some(sheet.clone());
			}
			if !state.1.contains(key) {
				break;
			}
			state = self.built.wait(state).unwrap();
		}
		// Not building: read it (once), or build it.
		match cache.read(key) {
			Some(sheet) => {
				let sheet = Arc::new(sheet);
				state.0.insert(key.to_string(), sheet.clone());
				Some(sheet)
			}
			None => {
				state.1.insert(key.to_string());
				None
			}
		}
	}

	/// Drop the kept sheet for `key` (it's damaged: it's built again).
	fn forget(&self, key: &str) {
		self.state.lock().unwrap().0.remove(key);
	}

	/// The build of `key` is over (its sheet written, or not).
	fn done(&self, cache: &SheetCache, key: &str) {
		let sheet = cache.read(key).map(Arc::new);
		let mut state = self.state.lock().unwrap();
		state.1.remove(key);
		if let Some(sheet) = sheet {
			state.0.insert(key.to_string(), sheet);
		}
		self.built.notify_all();
	}
}

/// Loads frames on worker threads.
pub struct FrameLoader {
	queue: Arc<Queue>,
	sheets: Arc<Sheets>,
	results: Receiver<LoadedFrames>,
	workers: Vec<JoinHandle<()>>,
}

impl FrameLoader {
	/// A loader reading atlases from `source`, keeping sheets in `cache` (none: every
	/// request decodes its atlas), with `threads` workers (at least one).
	pub fn new(source: Arc<dyn FileSource>, cache: Option<SheetCache>, threads: usize) -> Self {
		let queue = Arc::new(Queue::default());
		let sheets = Arc::new(Sheets::default());
		let (sender, results) = channel();
		let workers = (0..threads.max(1))
			.map(|i| {
				let (source, cache, queue, sheets, sender) =
					(source.clone(), cache.clone(), queue.clone(), sheets.clone(), sender.clone());
				std::thread::Builder::new()
					.name(format!("frame-loader-{i}"))
					.spawn(move || work(&*source, cache.as_ref(), &queue, &sheets, &sender))
					.expect("spawn frame loader thread")
			})
			.collect();
		FrameLoader {
			queue,
			sheets,
			results,
			workers,
		}
	}

	pub fn request(&self, request: FrameRequest) {
		self.queue.jobs.lock().unwrap().0.push_back(request);
		self.queue.ready.notify_one();
	}

	/// A finished request, if any is waiting.
	pub fn try_take(&self) -> Option<LoadedFrames> {
		self.results.try_recv().ok()
	}

	/// The next finished request, waiting for one until `deadline`.
	pub fn take_until(&self, deadline: std::time::Instant) -> Option<LoadedFrames> {
		let left = deadline.checked_duration_since(std::time::Instant::now())?;
		self.results.recv_timeout(left).ok()
	}

	/// Load `request` on this thread now.
	pub fn load_now(&self, source: &dyn FileSource, cache: Option<&SheetCache>, request: &FrameRequest) -> Result<Vec<LoadedFrame>, String> {
		load(source, cache, &self.sheets, request)
	}
}

impl Drop for FrameLoader {
	fn drop(&mut self) {
		self.queue.jobs.lock().unwrap().1 = true;
		self.queue.ready.notify_all();
		for worker in self.workers.drain(..) {
			let _ = worker.join();
		}
	}
}

fn work(source: &dyn FileSource, cache: Option<&SheetCache>, queue: &Queue, sheets: &Sheets, results: &Sender<LoadedFrames>) {
	loop {
		let request = {
			let mut jobs = queue.jobs.lock().unwrap();
			loop {
				if jobs.1 {
					return;
				}
				if let Some(request) = jobs.0.pop_front() {
					break request;
				}
				jobs = queue.ready.wait(jobs).unwrap();
			}
		};
		let frames = load(source, cache, sheets, &request);
		if results.send((request.atlas, frames)).is_err() {
			return;
		}
	}
}

/// The sheet's key: the atlas's identity in the game's files and its frames.
fn key(source: &dyn FileSource, request: &FrameRequest) -> Option<String> {
	let identity = source.identity(&request.atlas)?;
	let mut frames = Vec::with_capacity(request.frames.len() * 16);
	for f in request.frames.iter() {
		for v in [f.x, f.y, f.w, f.h] {
			frames.extend_from_slice(&v.to_le_bytes());
		}
	}
	Some(format!("{}|{identity}|{:016x}", request.atlas, fnv1a(&frames)))
}

fn load(source: &dyn FileSource, cache: Option<&SheetCache>, sheets: &Sheets, request: &FrameRequest) -> Result<Vec<LoadedFrame>, String> {
	let key = cache.and_then(|_| key(source, request));
	let Some((cache, key)) = cache.zip(key) else {
		return cut_from_atlas(source, None, request);
	};
	if let Some(sheet) = sheets.get(cache, &key) {
		let read = sheet.read_frames(&request.wanted).map(|frames| {
			frames
				.into_iter()
				.map(|(entry, levels)| LoadedFrame {
					frame: entry.frame,
					region: entry.region,
					levels,
				})
				.collect()
		});
		if let Some(frames) = read {
			return Ok(frames);
		}
		// A damaged sheet: built again (a rename replaces it whole).
		sheets.forget(&key);
	}
	let frames = cut_from_atlas(source, Some((cache, &key)), request);
	sheets.done(cache, &key);
	frames
}

/// `request`'s frames cut from its atlas, decoded; with a cache, every frame's region
/// kept there as the sheet for `key`.
fn cut_from_atlas(source: &dyn FileSource, cache: Option<(&SheetCache, &String)>, request: &FrameRequest) -> Result<Vec<LoadedFrame>, String> {
	let chain = mip_levels(&load_image(source, &request.atlas)?, LEVELS);
	let regions: HashMap<TexelRect, (Region, Vec<Image>)> = request
		.frames
		.iter()
		.map(|frame| {
			let region = frame.region();
			(*frame, (region, cut_levels(&chain, region)))
		})
		.collect();
	if let Some((cache, key)) = cache {
		let all: Vec<(TexelRect, Region, &[Image])> = request
			.frames
			.iter()
			.map(|frame| (*frame, regions[frame].0, &regions[frame].1[..]))
			.collect();
		let _ = cache.write(key, &all);
	}
	Ok(request
		.wanted
		.iter()
		.filter_map(|frame| {
			let (region, levels) = regions.get(frame)?;
			Some(LoadedFrame {
				frame: *frame,
				region: *region,
				levels: levels.clone(),
			})
		})
		.collect())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::time::{Duration, Instant};

	struct Files {
		atlas: Vec<u8>,
		reads: AtomicUsize,
	}

	impl FileSource for Files {
		fn read(&self, path: &str) -> Option<Vec<u8>> {
			(path == "gfx/a.png").then(|| {
				self.reads.fetch_add(1, Ordering::SeqCst);
				self.atlas.clone()
			})
		}

		fn identity(&self, path: &str) -> Option<String> {
			(path == "gfx/a.png").then(|| "test.dat|1234|5".to_string())
		}
	}

	fn atlas() -> Image {
		Image {
			width: 32,
			height: 32,
			rgba: (0..32 * 32 * 4).map(|i| (i % 251) as u8).collect(),
		}
	}

	fn png_of(image: &Image) -> Vec<u8> {
		crate::image::tests::encode(image.width, image.height, png::ColorType::Rgba, &image.rgba, None)
	}

	fn wait(loader: &FrameLoader) -> LoadedFrames {
		let deadline = Instant::now() + Duration::from_secs(10);
		loop {
			if let Some(result) = loader.try_take() {
				return result;
			}
			assert!(Instant::now() < deadline, "no answer");
			std::thread::sleep(Duration::from_millis(1));
		}
	}

	#[test]
	fn the_atlas_is_decoded_once_then_frames_come_from_the_sheet() {
		let dir = std::env::temp_dir().join(format!("bb-frame-loader-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		let source = Arc::new(Files {
			atlas: png_of(&atlas()),
			reads: AtomicUsize::new(0),
		});
		let loader = FrameLoader::new(source.clone(), Some(SheetCache::at(&dir)), 1);
		let frames: Arc<[TexelRect]> = Arc::new([TexelRect { x: 9, y: 9, w: 4, h: 4 }, TexelRect { x: 20, y: 2, w: 6, h: 6 }]);
		let request = |wanted: Vec<TexelRect>| FrameRequest {
			atlas: "gfx/a.png".into(),
			frames: frames.clone(),
			wanted,
		};
		// A warm-up: the sheet is filled, nothing is sent back.
		loader.request(request(Vec::new()));
		assert_eq!(wait(&loader), ("gfx/a.png".to_string(), Ok(Vec::new())));
		assert_eq!(source.reads.load(Ordering::SeqCst), 1);
		// Then a frame comes from the sheet, without reading the atlas again.
		loader.request(request(vec![frames[1]]));
		let (_, loaded) = wait(&loader);
		let loaded = loaded.expect("loaded");
		assert_eq!(source.reads.load(Ordering::SeqCst), 1, "from the sheet");
		let region = frames[1].region();
		assert_eq!((loaded[0].frame, loaded[0].region), (frames[1], region));
		let chain = mip_levels(&atlas(), LEVELS);
		assert_eq!(loaded[0].levels, cut_levels(&chain, region), "the atlas's texels, and its levels'");
		let _ = std::fs::remove_dir_all(dir);
	}

	#[test]
	fn requests_at_once_on_a_cold_cache_decode_the_atlas_once() {
		let dir = std::env::temp_dir().join(format!("bb-frame-loader-once-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		let source = Arc::new(Files {
			atlas: png_of(&atlas()),
			reads: AtomicUsize::new(0),
		});
		let loader = FrameLoader::new(source.clone(), Some(SheetCache::at(&dir)), 4);
		let frames: Arc<[TexelRect]> = Arc::new([TexelRect { x: 9, y: 9, w: 4, h: 4 }, TexelRect { x: 20, y: 2, w: 6, h: 6 }]);
		for i in 0..4 {
			loader.request(FrameRequest {
				atlas: "gfx/a.png".into(),
				frames: frames.clone(),
				wanted: vec![frames[i % 2]],
			});
		}
		for _ in 0..4 {
			let (_, loaded) = wait(&loader);
			assert_eq!(loaded.expect("loaded").len(), 1);
		}
		assert_eq!(source.reads.load(Ordering::SeqCst), 1, "one worker built the sheet, the rest read it");
		let _ = std::fs::remove_dir_all(dir);
	}

	#[test]
	fn without_a_cache_every_request_decodes() {
		let source = Arc::new(Files {
			atlas: png_of(&atlas()),
			reads: AtomicUsize::new(0),
		});
		let loader = FrameLoader::new(source.clone(), None, 1);
		let frames: Arc<[TexelRect]> = Arc::new([TexelRect { x: 9, y: 9, w: 4, h: 4 }]);
		for _ in 0..2 {
			loader.request(FrameRequest {
				atlas: "gfx/a.png".into(),
				frames: frames.clone(),
				wanted: vec![frames[0]],
			});
			assert!(wait(&loader).1.is_ok());
		}
		assert_eq!(source.reads.load(Ordering::SeqCst), 2);
		loader.request(FrameRequest {
			atlas: "gfx/missing.png".into(),
			frames: frames.clone(),
			wanted: vec![frames[0]],
		});
		assert!(wait(&loader).1.is_err());
	}
}
