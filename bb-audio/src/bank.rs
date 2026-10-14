//! The game's sounds, read from its files the first time they play (no preload
//! lists).
//!
//! A short sound is decoded once, whole, and kept: the next play of it starts from
//! memory. A long one (music, the ambience loops) keeps only its file's bytes and is
//! decoded as it plays, on kira's side, so a 13 MB rain loop costs no decoding on the
//! game's thread. A file that's missing or doesn't decode is remembered as such, and
//! reported once.
//!
//! The decoded sounds kept are capped: past [`KEEP_BYTES`], the least recently played
//! go (and are read again if they play again). The original keeps a sound only while
//! one of its instances plays (`0x5283e0`), so this keeps more, not less.
//!
//! A sound known to be wanted soon (the next music track, a new ambience layer's
//! sounds) is [`prefetch`](SoundBank::prefetch)ed: read, and decoded if short, on a
//! thread of its own. Its first use then waits only for what's left, as the
//! original's player waits for a track it loads asynchronously. A first use without
//! a prefetch reads on the caller's thread (a music track about 30 ms, the biggest
//! ambience loop 15 ms, an effect well under 1 ms; measured 2026-10-14).

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::thread::JoinHandle;

use bb_assets::FileSource;
use kira::sound::FromFileError;
use kira::sound::static_sound::StaticSoundData;
use kira::sound::streaming::StreamingSoundData;
use squirrel_bb::sound_length_ms;

/// Files at least this large stream rather than decode whole.
const STREAM_FROM: usize = 1 << 20;

/// How many bytes of decoded sound the bank keeps at most.
pub const KEEP_BYTES: usize = 64 << 20;

/// A decoded frame's size (two `f32` channels).
const FRAME_BYTES: usize = 8;

/// A sound ready to play.
pub enum Sound {
	Static(StaticSoundData),
	Streaming(StreamingSoundData<FromFileError>),
}

enum Entry {
	Static(StaticSoundData, u32),
	/// The file's bytes, decoded as it plays, and its length.
	Bytes(Arc<[u8]>, u32),
	Missing,
}

pub struct SoundBank {
	files: Arc<dyn FileSource>,
	/// Each entry and when it was last used.
	entries: HashMap<String, (Entry, u64)>,
	uses: u64,
	/// Bytes of decoded sound kept.
	kept: usize,
	/// Sounds being read on threads of their own.
	pending: HashMap<String, JoinHandle<Entry>>,
}

impl SoundBank {
	pub fn new(files: Arc<dyn FileSource>) -> SoundBank {
		SoundBank {
			files,
			entries: HashMap::new(),
			uses: 0,
			kept: 0,
			pending: HashMap::new(),
		}
	}

	/// The sound at `path`, read and (if short) decoded on its first use; `None` if
	/// there's no such file or it doesn't decode.
	pub fn get(&mut self, path: &str) -> Option<Sound> {
		match self.entry(path) {
			Entry::Static(data, _) => Some(Sound::Static(data.clone())),
			Entry::Bytes(bytes, _) => match StreamingSoundData::from_cursor(Cursor::new(bytes.clone())) {
				Ok(data) => Some(Sound::Streaming(data)),
				Err(e) => {
					eprintln!("audio: {path}: {e}");
					None
				}
			},
			Entry::Missing => None,
		}
	}

	/// How long the sound at `path` plays at normal speed, in milliseconds (0 for
	/// none), read on its first use like [`get`](Self::get).
	pub fn length_ms(&mut self, path: &str) -> u32 {
		match self.entry(path) {
			Entry::Static(_, ms) | Entry::Bytes(_, ms) => *ms,
			Entry::Missing => 0,
		}
	}

	/// Whether `path` has been read (and, if short, decoded).
	pub fn is_loaded(&self, path: &str) -> bool {
		self.entries.contains_key(path)
	}

	/// Start reading (and, if short, decoding) `path` on a thread of its own, unless
	/// it's read or being read already.
	pub fn prefetch(&mut self, path: &str) {
		if self.entries.contains_key(path) || self.pending.contains_key(path) {
			return;
		}
		let (files, owned) = (self.files.clone(), path.to_string());
		let handle = std::thread::Builder::new()
			.name("sound loader".into())
			.spawn(move || load(&*files, &owned));
		match handle {
			Ok(handle) => {
				self.pending.insert(path.to_string(), handle);
			}
			// No thread: it loads on first use instead.
			Err(e) => eprintln!("audio: {path}: no loader thread: {e}"),
		}
	}

	/// Bytes of decoded sound kept.
	pub fn kept_bytes(&self) -> usize {
		self.kept
	}

	fn entry(&mut self, path: &str) -> &Entry {
		self.uses += 1;
		let now = self.uses;
		if let Some((_, used)) = self.entries.get_mut(path) {
			*used = now;
		} else {
			let entry = match self.pending.remove(path) {
				Some(loading) => loading.join().unwrap_or(Entry::Missing),
				None => load(&*self.files, path),
			};
			self.kept += decoded_bytes(&entry);
			self.entries.insert(path.to_string(), (entry, now));
			self.trim(path);
		}
		&self.entries[path].0
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
fn load(files: &dyn FileSource, path: &str) -> Entry {
	let Some(bytes) = files.read(path) else {
		eprintln!("audio: {path}: no such file");
		return Entry::Missing;
	};
	let length = sound_length_ms(&bytes).unwrap_or(0);
	if bytes.len() >= STREAM_FROM {
		return Entry::Bytes(bytes.into(), length);
	}
	match StaticSoundData::from_cursor(Cursor::new(bytes)) {
		Ok(data) => Entry::Static(data, length),
		Err(e) => {
			eprintln!("audio: {path}: {e}");
			Entry::Missing
		}
	}
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

	/// A 0.1 s mono wav, and which threads read files.
	struct Files(Mutex<Vec<String>>);

	impl FileSource for Files {
		fn read(&self, path: &str) -> Option<Vec<u8>> {
			let thread = std::thread::current().name().unwrap_or("").to_string();
			self.0.lock().unwrap().push(format!("{path} on {thread}"));
			let frames = 4410u32;
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
			(path == "a.wav").then_some(b)
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
}
