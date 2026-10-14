//! The game's sounds, read from its files the first time they play (no preload
//! lists).
//!
//! A short sound is decoded once, whole, and kept: the next play of it starts from
//! memory. A long one (music, the ambience loops) is decoded as it plays, on kira's
//! side, so a 13 MB rain loop costs no decoding on the game's thread: read in place from
//! its archive when the archive stores it uncompressed (all of vanilla's do), or else
//! from its file's bytes. A file that's missing or doesn't decode is remembered as such,
//! and reported once.
//!
//! What stays in memory: the original keeps a sound only while one of its instances
//! plays (`0x5283e0`).
//! - A long sound read in place keeps nothing but its length; its voices read the archive
//!   as they play.
//! - A long sound read whole (a mod's compressed file) is held by the voices playing it,
//!   as the original's, and goes when the last one ends; the bank remembers only its
//!   length, and reads it again for its next play (on a loader thread when it's
//!   prefetched, as the next track is). Holding them all, every track and ambience loop
//!   ever played stayed in memory, up to about 730 MB for vanilla's.
//! - Decoded short sounds are kept beyond their play, as a cache, up to [`KEEP_BYTES`]:
//!   past it the least recently played go (and are read again if they play again).
//! - What's read ahead and not yet played is kept up to [`KEEP_READ_AHEAD`], the oldest
//!   going first, and a scene the game discards (a battle's end, the campaign left, a
//!   load) takes it, and the decoded sounds: an ambience layer prefetches all its
//!   sounds (its loops a few MB each) and plays some, and the music's next track changes
//!   with the scene. Kept, those piled up: 159 MB of files and 66 MB of decoded sound
//!   after 6 battles against as many peoples and beasts.
//!
//! A sound known to be wanted soon (the next music track, a new ambience layer's
//! sounds) is [`prefetch`](SoundBank::prefetch)ed: read, and decoded if short, on a
//! thread of its own. Its first use then waits only for what's left, as the
//! original's player waits for a track it loads asynchronously. A first use without
//! a prefetch reads on the caller's thread (a music track about 30 ms, the biggest
//! ambience loop 15 ms, an effect well under 1 ms; measured 2026-10-14).

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;

use bb_assets::{FileSource, FileStream};
use kira::sound::FromFileError;
use kira::sound::static_sound::StaticSoundData;
use kira::sound::streaming::StreamingSoundData;
use squirrel_bb::{sound_length_ms, sound_length_ms_in};
use symphonia::core::io::MediaSource;

/// Files at least this large stream rather than decode whole.
const STREAM_FROM: usize = 1 << 20;

/// How many bytes of decoded sound the bank keeps at most: about 95 s of sound, more than
/// a scene plays (a battle's decoded 27–36 MB over 8 battles; it starts anew at each
/// discarded scene), and a short sound decodes again in well under a millisecond.
pub const KEEP_BYTES: usize = 32 << 20;

/// The bytes read ahead and not yet played kept at most (files of long sounds, decoded
/// short ones): the next track and some of a layer's sounds. Past it the oldest go, and
/// their first play reads them on the game's thread (a track ~30 ms, a short sound well
/// under 1 ms).
const KEEP_READ_AHEAD: usize = 16 << 20;

/// A decoded frame's size (two `f32` channels).
const FRAME_BYTES: usize = 8;

/// A sound ready to play.
pub enum Sound {
	Static(StaticSoundData),
	Streaming(StreamingSoundData<FromFileError>),
}

/// What the bank knows of a sound.
enum Entry {
	Static(StaticSoundData, u32),
	/// A long sound read in place from its archive: its length.
	InPlace(u32),
	/// A long sound's file, while a voice holds it, and its length.
	Bytes(Weak<Box<[u8]>>, u32),
	Missing,
}

/// A sound read from the files.
enum Loaded {
	Static(StaticSoundData, u32),
	/// A long sound read in place from its archive as it plays: its length.
	InPlace(u32),
	/// A long sound's file, decoded as it plays, and its length.
	Bytes(Arc<Box<[u8]>>, u32),
	Missing,
}

/// A long sound's file in its archive, as symphonia reads its media.
struct InPlace(Box<dyn FileStream>);

impl Read for InPlace {
	fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
		self.0.read(out)
	}
}

impl Seek for InPlace {
	fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
		self.0.seek(to)
	}
}

impl MediaSource for InPlace {
	fn is_seekable(&self) -> bool {
		true
	}

	fn byte_len(&self) -> Option<u64> {
		Some(self.0.len())
	}
}

/// A long sound's file, shared by the voices playing it. Boxed inside the `Arc`, so the
/// bank's `Weak` holds only the `Arc`'s own few bytes once the last voice is done: an
/// `Arc<[u8]>` keeps the file in its own block, which a `Weak` keeps allocated (every
/// long sound ever played stayed in memory that way: 146 MB after 6 battles).
struct FileBytes(Arc<Box<[u8]>>);

impl AsRef<[u8]> for FileBytes {
	fn as_ref(&self) -> &[u8] {
		&self.0
	}
}

pub struct SoundBank {
	files: Arc<dyn FileSource>,
	/// Each entry and when it was last used.
	entries: HashMap<String, (Entry, u64)>,
	uses: u64,
	/// Bytes of decoded sound kept.
	kept: usize,
	/// Sounds being read on threads of their own, and when each was asked for.
	pending: HashMap<String, (JoinHandle<Loaded>, u64)>,
	/// Sounds read ahead, not yet played: when each was asked for, and its bytes.
	read_ahead: HashMap<String, (Loaded, u64, usize)>,
}

impl SoundBank {
	pub fn new(files: Arc<dyn FileSource>) -> SoundBank {
		SoundBank {
			files,
			entries: HashMap::new(),
			uses: 0,
			kept: 0,
			pending: HashMap::new(),
			read_ahead: HashMap::new(),
		}
	}

	/// The sound at `path`, read and (if short) decoded on its first use; `None` if
	/// there's no such file or it doesn't decode.
	pub fn get(&mut self, path: &str) -> Option<Sound> {
		match self.entry(path) {
			Loaded::Static(data, _) => Some(Sound::Static(data)),
			// The voice holds the bytes from here on.
			Loaded::InPlace(_) => {
				let Some(file) = self.files.stream(path) else {
					eprintln!("audio: {path}: no longer in its archive");
					return None;
				};
				match StreamingSoundData::from_media_source(InPlace(file)) {
					Ok(data) => Some(Sound::Streaming(data)),
					Err(e) => {
						eprintln!("audio: {path}: {e}");
						None
					}
				}
			}
			Loaded::Bytes(bytes, _) => match StreamingSoundData::from_cursor(Cursor::new(FileBytes(bytes))) {
				Ok(data) => Some(Sound::Streaming(data)),
				Err(e) => {
					eprintln!("audio: {path}: {e}");
					None
				}
			},
			Loaded::Missing => None,
		}
	}

	/// How long the sound at `path` plays at normal speed, in milliseconds (0 for
	/// none), read on its first use like [`get`](Self::get).
	pub fn length_ms(&mut self, path: &str) -> u32 {
		if let Some((Entry::Static(_, ms) | Entry::InPlace(ms) | Entry::Bytes(_, ms), _)) = self.entries.get(path) {
			return *ms;
		}
		match self.entry(path) {
			Loaded::Static(_, ms) | Loaded::InPlace(ms) | Loaded::Bytes(_, ms) => ms,
			Loaded::Missing => 0,
		}
	}

	/// Whether `path` is in memory: decoded (if short), held by a voice (if long), or
	/// known to be missing.
	pub fn is_loaded(&self, path: &str) -> bool {
		match self.entries.get(path) {
			Some((Entry::Bytes(bytes, _), _)) => bytes.strong_count() > 0,
			Some(_) => true,
			None => false,
		}
	}

	/// Start reading (and, if short, decoding) `path` on a thread of its own, unless
	/// it's read or being read already.
	pub fn prefetch(&mut self, path: &str) {
		if self.is_loaded(path) || self.pending.contains_key(path) || self.read_ahead.contains_key(path) {
			return;
		}
		let (files, owned) = (self.files.clone(), path.to_string());
		let handle = std::thread::Builder::new()
			.name("sound loader".into())
			.spawn(move || load(&*files, &owned));
		match handle {
			Ok(handle) => {
				self.uses += 1;
				self.pending.insert(path.to_string(), (handle, self.uses));
			}
			// No thread: it loads on first use instead.
			Err(e) => eprintln!("audio: {path}: no loader thread: {e}"),
		}
	}

	/// Bytes of decoded sound kept.
	pub fn kept_bytes(&self) -> usize {
		self.kept
	}

	/// Drop the decoded sounds kept for reuse: the game discarded a scene (a battle
	/// ended, a campaign was left), whose sounds rarely play in the next. A voice playing
	/// one keeps its own copy.
	pub fn discard_unused(&mut self) {
		self.entries.retain(|_, (entry, _)| !matches!(entry, Entry::Static(..)));
		self.kept = 0;
		// Prefetches still reading finish, and are kept as read ahead from then on.
		self.settle();
		self.read_ahead.clear();
	}

	/// A frame passed: take in the prefetches that finished, keeping what's read ahead
	/// within [`KEEP_READ_AHEAD`].
	pub fn tick(&mut self) {
		self.settle();
	}

	/// Move the finished prefetches to what's read ahead, and drop the oldest of it past
	/// [`KEEP_READ_AHEAD`].
	fn settle(&mut self) {
		let finished: Vec<String> = self
			.pending
			.iter()
			.filter(|(_, (loading, _))| loading.is_finished())
			.map(|(path, _)| path.clone())
			.collect();
		for path in finished {
			let Some((loading, asked)) = self.pending.remove(&path) else { continue };
			let loaded = loading.join().unwrap_or(Loaded::Missing);
			let bytes = match &loaded {
				Loaded::Static(data, _) => data.num_frames() * FRAME_BYTES,
				Loaded::Bytes(bytes, _) => bytes.len(),
				Loaded::InPlace(_) => 0,
				Loaded::Missing => 0,
			};
			self.read_ahead.insert(path, (loaded, asked, bytes));
		}
		let mut held: usize = self.read_ahead.values().map(|(_, _, bytes)| bytes).sum();
		while held > KEEP_READ_AHEAD {
			let oldest = self.read_ahead.iter().min_by_key(|(_, (_, asked, _))| *asked).map(|(path, _)| path.clone());
			let Some((_, _, bytes)) = oldest.and_then(|path| self.read_ahead.remove(&path)) else { break };
			held -= bytes;
		}
	}

	/// What the bank holds: the sounds it knows, the bytes of decoded sound, the bytes of
	/// long sounds the voices playing them hold, and the bytes read ahead, not yet played.
	pub fn memory_use(&self) -> (usize, usize, usize, usize) {
		let streamed = self
			.entries
			.values()
			.filter_map(|(entry, _)| match entry {
				Entry::Bytes(bytes, _) => bytes.upgrade().map(|b| b.len()),
				_ => None,
			})
			.sum();
		let read_ahead = self.read_ahead.values().map(|(_, _, bytes)| bytes).sum();
		(self.entries.len(), self.kept, streamed, read_ahead)
	}

	/// The sound at `path`: kept, or read (the prefetch's if one ran) and remembered.
	fn entry(&mut self, path: &str) -> Loaded {
		self.uses += 1;
		let now = self.uses;
		if let Some((entry, used)) = self.entries.get_mut(path) {
			let kept = match entry {
				Entry::Static(data, ms) => Some(Loaded::Static(data.clone(), *ms)),
				Entry::InPlace(ms) => Some(Loaded::InPlace(*ms)),
				Entry::Bytes(bytes, ms) => bytes.upgrade().map(|bytes| Loaded::Bytes(bytes, *ms)),
				Entry::Missing => Some(Loaded::Missing),
			};
			if let Some(kept) = kept {
				*used = now;
				return kept;
			}
		}
		let read = self.read_ahead.remove(path).map(|(loaded, _, _)| loaded);
		let loaded = match read.or_else(|| self.pending.remove(path).map(|(loading, _)| loading.join().unwrap_or(Loaded::Missing))) {
			Some(loaded) => loaded,
			None => load(&*self.files, path),
		};
		let entry = match &loaded {
			Loaded::Static(data, ms) => Entry::Static(data.clone(), *ms),
			Loaded::InPlace(ms) => Entry::InPlace(*ms),
			Loaded::Bytes(bytes, ms) => Entry::Bytes(Arc::downgrade(bytes), *ms),
			Loaded::Missing => Entry::Missing,
		};
		self.kept += decoded_bytes(&entry);
		self.entries.insert(path.to_string(), (entry, now));
		self.trim(path);
		loaded
	}

	/// Drop the least recently used decoded sounds (but `keep`) past the cap.
	fn trim(&mut self, keep: &str) {
		while self.kept > KEEP_BYTES {
			let oldest = self
				.entries
				.iter()
				.filter(|(path, (entry, _))| path.as_str() != keep && decoded_bytes(entry) > 0)
				.min_by_key(|(_, (_, used))| *used)
				.map(|(path, _)| path.clone());
			let Some(oldest) = oldest else { break };
			if let Some((entry, _)) = self.entries.remove(&oldest) {
				self.kept -= decoded_bytes(&entry);
			}
		}
	}

}

/// Read `path` from `files`, decoding it if short.
fn load(files: &dyn FileSource, path: &str) -> Loaded {
	if let Some(mut file) = files.stream(path).filter(|f| f.len() >= STREAM_FROM as u64) {
		return match in_place_length_ms(&mut *file) {
			Some(length) => Loaded::InPlace(length),
			None => {
				eprintln!("audio: {path}: not a sound");
				Loaded::Missing
			}
		};
	}
	let Some(bytes) = files.read(path) else {
		eprintln!("audio: {path}: no such file");
		return Loaded::Missing;
	};
	let length = sound_length_ms(&bytes).unwrap_or(0);
	if bytes.len() >= STREAM_FROM {
		return Loaded::Bytes(Arc::new(bytes.into_boxed_slice()), length);
	}
	match StaticSoundData::from_cursor(Cursor::new(bytes)) {
		Ok(data) => Loaded::Static(data, length),
		Err(e) => {
			eprintln!("audio: {path}: {e}");
			Loaded::Missing
		}
	}
}

/// A long sound's length, from the start and the end of its file (see
/// [`sound_length_ms_in`]).
fn in_place_length_ms(file: &mut dyn FileStream) -> Option<u32> {
	const SPAN: u64 = 64 << 10;
	let len = file.len();
	let mut head = Vec::new();
	file.seek(SeekFrom::Start(0)).ok()?;
	(&mut *file).take(SPAN).read_to_end(&mut head).ok()?;
	let mut tail = Vec::new();
	file.seek(SeekFrom::Start(len.saturating_sub(SPAN))).ok()?;
	(&mut *file).take(SPAN).read_to_end(&mut tail).ok()?;
	file.seek(SeekFrom::Start(0)).ok()?;
	sound_length_ms_in(&head, &tail, len)
}

fn decoded_bytes(entry: &Entry) -> usize {
	match entry {
		Entry::Static(data, _) => data.num_frames() * FRAME_BYTES,
		_ => 0,
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Mutex;

	use super::*;

	/// A 0.1 s mono wav (`a.wav`), a 15 s one (`long.wav`, streamed), and which threads
	/// read files.
	struct Files(Mutex<Vec<String>>);

	impl FileSource for Files {
		fn read(&self, path: &str) -> Option<Vec<u8>> {
			let thread = std::thread::current().name().unwrap_or("").to_string();
			self.0.lock().unwrap().push(format!("{path} on {thread}"));
			let frames = if path == "long.wav" { 661_500u32 } else { 4410 };
			let mut b = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0".to_vec();
			b.extend_from_slice(&44_100u32.to_le_bytes());
			b.extend_from_slice(&88_200u32.to_le_bytes());
			b.extend_from_slice(&2u16.to_le_bytes());
			b.extend_from_slice(&16u16.to_le_bytes());
			b.extend_from_slice(b"data");
			b.extend_from_slice(&(frames * 2).to_le_bytes());
			b.resize(b.len() + frames as usize * 2, 0);
			let riff = (b.len() - 8) as u32;
			b[4..8].copy_from_slice(&riff.to_le_bytes());
			(path == "a.wav" || path == "long.wav").then_some(b)
		}
	}

	#[test]
	fn a_prefetched_sound_is_read_once_on_the_loader_thread() {
		let files = Arc::new(Files(Mutex::new(Vec::new())));
		let mut bank = SoundBank::new(files.clone());
		bank.prefetch("a.wav");
		bank.prefetch("a.wav");
		assert!(bank.get("a.wav").is_some());
		assert_eq!(bank.length_ms("a.wav"), 100);
		assert_eq!(*files.0.lock().unwrap(), ["a.wav on sound loader"]);
		assert!(bank.get("missing.wav").is_none());
	}

	/// [`Files`], whose archive stores its files uncompressed: they can be read in place.
	struct Stored(Files);

	struct InMemory(std::io::Cursor<Vec<u8>>);

	impl Read for InMemory {
		fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
			self.0.read(out)
		}
	}

	impl Seek for InMemory {
		fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
			self.0.seek(to)
		}
	}

	impl FileStream for InMemory {
		fn len(&self) -> u64 {
			self.0.get_ref().len() as u64
		}
	}

	impl FileSource for Stored {
		fn read(&self, path: &str) -> Option<Vec<u8>> {
			self.0.read(path)
		}

		fn stream(&self, path: &str) -> Option<Box<dyn FileStream>> {
			let bytes = self.0.read(path)?;
			self.0 .0.lock().unwrap().push(format!("{path} streamed"));
			Some(Box::new(InMemory(std::io::Cursor::new(bytes))))
		}
	}

	#[test]
	fn a_long_sound_in_an_uncompressed_archive_plays_in_place() {
		let files = Arc::new(Stored(Files(Mutex::new(Vec::new()))));
		let mut bank = SoundBank::new(files.clone());
		assert_eq!(bank.length_ms("long.wav"), 15_000, "from the file's start and end");
		let playing = bank.get("long.wav");
		assert!(matches!(playing, Some(Sound::Streaming(_))));
		assert_eq!(bank.memory_use().2, 0, "none of its file held: the voice reads it in place");
		assert!(bank.is_loaded("long.wav"), "known: nothing to read ahead");
		// A short sound is still read whole and decoded.
		assert!(matches!(bank.get("a.wav"), Some(Sound::Static(_))));
	}

	#[test]
	fn a_long_sound_stays_only_while_it_plays() {
		let files = Arc::new(Files(Mutex::new(Vec::new())));
		let mut bank = SoundBank::new(files.clone());
		let playing = bank.get("long.wav");
		assert!(matches!(playing, Some(Sound::Streaming(_))));
		assert_eq!(bank.memory_use().2, 1_323_044, "the playing voice holds its bytes");
		assert!(bank.is_loaded("long.wav"));
		drop(playing);
		assert_eq!(bank.memory_use().2, 0, "gone with the voice");
		assert!(!bank.is_loaded("long.wav"));
		assert_eq!(bank.length_ms("long.wav"), 15_000, "its length is remembered");
		assert_eq!(files.0.lock().unwrap().len(), 1, "without reading it again");
		assert!(bank.get("long.wav").is_some());
		assert_eq!(files.0.lock().unwrap().len(), 2, "read again to play again");
	}

	#[test]
	fn a_discarded_scene_takes_the_decoded_sounds_with_it() {
		let files = Arc::new(Files(Mutex::new(Vec::new())));
		let mut bank = SoundBank::new(files.clone());
		let playing = bank.get("a.wav");
		assert_eq!(bank.memory_use().1, 4410 * FRAME_BYTES);
		bank.discard_unused();
		assert_eq!(bank.memory_use(), (0, 0, 0, 0));
		assert!(matches!(playing, Some(Sound::Static(_))), "the playing voice keeps its copy");
		assert!(bank.get("a.wav").is_some());
		assert_eq!(files.0.lock().unwrap().len(), 2, "decoded again for its next play");
	}

	#[test]
	fn a_sound_read_ahead_plays_without_reading_again() {
		let files = Arc::new(Files(Mutex::new(Vec::new())));
		let mut bank = SoundBank::new(files.clone());
		bank.prefetch("long.wav");
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
		while bank.memory_use().3 == 0 {
			assert!(std::time::Instant::now() < deadline, "the loader never finished");
			bank.tick();
		}
		assert!(bank.get("long.wav").is_some());
		assert_eq!(bank.memory_use().3, 0, "no longer ahead: playing");
		assert_eq!(files.0.lock().unwrap().len(), 1, "read once, on the loader");
	}

	#[test]
	fn a_discarded_scene_takes_the_prefetches_nothing_played() {
		let files = Arc::new(Files(Mutex::new(Vec::new())));
		let mut bank = SoundBank::new(files.clone());
		bank.prefetch("long.wav");
		bank.prefetch("a.wav");
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
		while bank.pending.values().any(|(loading, _)| !loading.is_finished()) {
			assert!(std::time::Instant::now() < deadline, "the loaders never finished");
			std::thread::yield_now();
		}
		bank.tick();
		assert_eq!(bank.memory_use().3, 1_323_044 + 4410 * FRAME_BYTES, "read ahead, not yet played");
		bank.discard_unused();
		assert_eq!(bank.memory_use(), (0, 0, 0, 0));
	}
}
