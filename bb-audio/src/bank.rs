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

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

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
}

impl SoundBank {
	pub fn new(files: Arc<dyn FileSource>) -> SoundBank {
		SoundBank {
			files,
			entries: HashMap::new(),
			uses: 0,
			kept: 0,
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
			let entry = self.load(path);
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

	fn load(&self, path: &str) -> Entry {
		let Some(bytes) = self.files.read(path) else {
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
}

fn decoded_bytes(entry: &Entry) -> usize {
	match entry {
		Entry::Static(data, _) => data.num_frames() * FRAME_BYTES,
		_ => 0,
	}
}
