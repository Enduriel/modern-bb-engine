//! Our own sprite sheets on disk (texture residency), cached like the compiled scripts.
//! The first time an atlas is decoded, every frame's region ([`crate::frames`]) is cut
//! and kept here, so a frame the renderer dropped comes back as a small read instead of
//! the atlas's decode.
//!
//! - **Location:** the engine's cache folder ([`crate::cache_dir`]), `sheets/` under it,
//!   one subfolder per format version; `BB_SHEET_CACHE` names another, or `off`.
//! - **Entries:** one file per atlas, named by a hash of its key: the atlas's identity in
//!   the game's files (the archive it comes from, the entry's CRC32 and size, see
//!   `FileSource::identity`) and its frames. A mod that replaces the atlas, or changes its
//!   brushes, makes another key.
//! - **Format:** a header with the full key, the frames' index (each frame, its region,
//!   where each of its levels is), then each region's levels as PNGs.
//! - **Validation:** a missing, foreign or truncated file is a miss; writes go to a
//!   temporary file renamed into place. Failures are never fatal: it's an optimisation.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::frames::{Region, TexelRect};
use crate::{Image, decode_png};

/// Bump when the file format changes: each version has its own folder.
/// 2: regions reach past the atlas's edge (its texels repeated).
/// 3: a region's first mip levels, cut from the atlas's.
const VERSION: u32 = 3;
const MAGIC: &[u8; 8] = b"BBSHEET\0";

/// Our sheets' cache folder.
#[derive(Clone, Debug)]
pub struct SheetCache {
	dir: PathBuf,
}

/// One frame in a cached sheet: its rectangle in the atlas, its region, and where each
/// of the region's levels is in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetEntry {
	pub frame: TexelRect,
	pub region: Region,
	levels: Vec<(u64, u64)>,
}

/// A cached sheet's index.
#[derive(Clone, Debug)]
pub struct Sheet {
	file: PathBuf,
	pub entries: Vec<SheetEntry>,
	/// Each frame's entry.
	by_frame: HashMap<TexelRect, usize>,
}

impl SheetCache {
	/// The cache in `root` (its version's folder in it).
	pub fn at(root: impl AsRef<Path>) -> SheetCache {
		SheetCache {
			dir: root.as_ref().join(format!("v{VERSION}")),
		}
	}

	/// The cache in its usual place, or where `BB_SHEET_CACHE` says (`off`: none).
	pub fn from_env() -> Option<SheetCache> {
		match std::env::var_os("BB_SHEET_CACHE") {
			Some(v) if v == "off" || v == "0" => None,
			Some(dir) if !dir.is_empty() => Some(SheetCache::at(dir)),
			_ => crate::cache_dir::engine_cache_dir().map(|dir| SheetCache::at(dir.join("sheets"))),
		}
	}

	/// Whether the cache has a file for `key` (unread: it may still turn out stale).
	pub fn has(&self, key: &str) -> bool {
		self.file_for(key).exists()
	}

	fn file_for(&self, key: &str) -> PathBuf {
		self.dir.join(format!("{:016x}.sheet", fnv1a(key.as_bytes())))
	}

	/// The sheet cached for `key`, or `None` (a miss).
	pub fn read(&self, key: &str) -> Option<Sheet> {
		let file = self.file_for(key);
		let f = File::open(&file).ok()?;
		let length = f.metadata().ok()?.len();
		// The index is many small fields: read through a buffer.
		let mut f = BufReader::new(f);
		let mut header = [0u8; 12];
		f.read_exact(&mut header).ok()?;
		if &header[..8] != MAGIC || u32::from_le_bytes(header[8..12].try_into().ok()?) != VERSION {
			return None;
		}
		let stored_key = read_bytes(&mut f, length)?;
		if stored_key != key.as_bytes() {
			return None;
		}
		let count = read_u32(&mut f)? as usize;
		let mut entries = Vec::with_capacity(count.min(1 << 16));
		for _ in 0..count {
			let mut values = [0u32; 8];
			for v in &mut values {
				*v = read_u32(&mut f)?;
			}
			let [x, y, w, h, rx, ry, rw, rh] = values;
			let frame = TexelRect { x, y, w, h };
			let region = Region {
				x: rx as i32,
				y: ry as i32,
				w: rw,
				h: rh,
			};
			let count = read_u32(&mut f)?;
			if count > 16 {
				return None;
			}
			let mut levels = Vec::with_capacity(count as usize);
			for _ in 0..count {
				let (offset, len) = (read_u64(&mut f)?, read_u64(&mut f)?);
				if offset.checked_add(len)? > length {
					return None;
				}
				levels.push((offset, len));
			}
			entries.push(SheetEntry { frame, region, levels });
		}
		let by_frame = entries.iter().enumerate().map(|(i, e)| (e.frame, i)).collect();
		Some(Sheet { file, entries, by_frame })
	}

	/// Keep the sheet of `frames` (each frame, its region, the region's levels) for `key`.
	pub fn write(&self, key: &str, frames: &[(TexelRect, Region, &[Image])]) -> io::Result<()> {
		std::fs::create_dir_all(&self.dir)?;
		let blobs: Vec<Vec<Vec<u8>>> = frames
			.iter()
			.map(|(_, _, levels)| levels.iter().map(encode).collect::<io::Result<_>>())
			.collect::<io::Result<_>>()?;
		let mut head = Vec::new();
		head.extend_from_slice(MAGIC);
		head.extend_from_slice(&VERSION.to_le_bytes());
		head.extend_from_slice(&(key.len() as u32).to_le_bytes());
		head.extend_from_slice(key.as_bytes());
		head.extend_from_slice(&(frames.len() as u32).to_le_bytes());
		let index_len: u64 = blobs.iter().map(|levels| 8 * 4 + 4 + 16 * levels.len() as u64).sum();
		let mut offset = head.len() as u64 + index_len;
		for ((frame, region, _), levels) in frames.iter().zip(&blobs) {
			for v in [frame.x, frame.y, frame.w, frame.h, region.x as u32, region.y as u32, region.w, region.h] {
				head.extend_from_slice(&v.to_le_bytes());
			}
			head.extend_from_slice(&(levels.len() as u32).to_le_bytes());
			for blob in levels {
				head.extend_from_slice(&offset.to_le_bytes());
				head.extend_from_slice(&(blob.len() as u64).to_le_bytes());
				offset += blob.len() as u64;
			}
		}
		let file = self.file_for(key);
		let temp = file.with_extension(format!("tmp{}", std::process::id()));
		{
			let mut f = io::BufWriter::new(File::create(&temp)?);
			f.write_all(&head)?;
			for blob in blobs.iter().flatten() {
				f.write_all(blob)?;
			}
			f.flush()?;
		}
		std::fs::rename(&temp, &file).inspect_err(|_| {
			let _ = std::fs::remove_file(&temp);
		})
	}
}

impl Sheet {
	/// The entry for `frame`, if the sheet has it.
	pub fn entry(&self, frame: &TexelRect) -> Option<&SheetEntry> {
		self.by_frame.get(frame).map(|&i| &self.entries[i])
	}

	/// `entry`'s region's levels, read and decoded, or `None` if the file no longer has
	/// them.
	pub fn read(&self, entry: &SheetEntry) -> Option<Vec<Image>> {
		Self::read_from(&mut File::open(&self.file).ok()?, entry)
	}

	/// The levels of each of `frames`' regions, in order, the file opened once; `None`
	/// if the sheet lacks one or the file no longer has it.
	pub fn read_frames(&self, frames: &[TexelRect]) -> Option<Vec<(&SheetEntry, Vec<Image>)>> {
		let mut f = File::open(&self.file).ok()?;
		frames
			.iter()
			.map(|frame| {
				let entry = self.entry(frame)?;
				Some((entry, Self::read_from(&mut f, entry)?))
			})
			.collect()
	}

	fn read_from(f: &mut File, entry: &SheetEntry) -> Option<Vec<Image>> {
		let mut levels = Vec::with_capacity(entry.levels.len());
		for (level, &(offset, len)) in entry.levels.iter().enumerate() {
			f.seek(SeekFrom::Start(offset)).ok()?;
			let mut bytes = vec![0; len as usize];
			f.read_exact(&mut bytes).ok()?;
			let image = decode_png(&bytes).ok()?;
			let size = |v: u32| (v >> level).max(1);
			if (image.width, image.height) != (size(entry.region.w), size(entry.region.h)) {
				return None;
			}
			levels.push(image);
		}
		Some(levels)
	}
}

/// `image` as a PNG, compressed quickly (the cache is written once, read often).
fn encode(image: &Image) -> io::Result<Vec<u8>> {
	let mut out = Vec::new();
	let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	encoder.set_compression(png::Compression::Fast);
	let mut writer = encoder.write_header().map_err(io::Error::other)?;
	writer.write_image_data(&image.rgba).map_err(io::Error::other)?;
	writer.finish().map_err(io::Error::other)?;
	Ok(out)
}

fn read_u32(f: &mut impl Read) -> Option<u32> {
	let mut b = [0u8; 4];
	f.read_exact(&mut b).ok()?;
	Some(u32::from_le_bytes(b))
}

fn read_u64(f: &mut impl Read) -> Option<u64> {
	let mut b = [0u8; 8];
	f.read_exact(&mut b).ok()?;
	Some(u64::from_le_bytes(b))
}

/// A length-prefixed byte string, no longer than the file.
fn read_bytes(f: &mut impl Read, limit: u64) -> Option<Vec<u8>> {
	let len = u64::from(read_u32(f)?);
	if len > limit {
		return None;
	}
	let mut bytes = vec![0; len as usize];
	f.read_exact(&mut bytes).ok()?;
	Some(bytes)
}

/// FNV-1a, 64-bit: a file name from a key.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
	bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn image(width: u32, height: u32, seed: u8) -> Image {
		Image {
			width,
			height,
			rgba: (0..width * height * 4).map(|i| (i as u8).wrapping_mul(seed)).collect(),
		}
	}

	fn scratch(name: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("bb-sheet-cache-{}-{name}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		dir
	}

	#[test]
	fn a_sheet_round_trips() {
		let dir = scratch("round-trip");
		let cache = SheetCache::at(&dir);
		let (a, b) = (image(16, 8, 3), image(24, 16, 5));
		let frame_a = TexelRect { x: 9, y: 9, w: 4, h: 2 };
		let region_a = Region { x: 0, y: -8, w: 16, h: 8 };
		let frame_b = TexelRect { x: 40, y: 8, w: 8, h: 4 };
		let region_b = Region { x: 32, y: 0, w: 24, h: 16 };
		assert!(cache.read("key").is_none(), "nothing yet");
		let (a1, b1) = (image(8, 4, 7), image(12, 8, 9));
		let (levels_a, levels_b) = ([a.clone(), a1.clone()], [b.clone(), b1.clone()]);
		cache.write("key", &[(frame_a, region_a, &levels_a[..]), (frame_b, region_b, &levels_b[..])]).expect("write");
		let sheet = cache.read("key").expect("cached");
		assert_eq!(sheet.entries.len(), 2);
		let entry = sheet.entry(&frame_b).expect("frame b");
		assert_eq!(entry.region, region_b);
		assert_eq!(sheet.read(entry), Some(vec![b, b1]));
		assert_eq!(sheet.read(sheet.entry(&frame_a).unwrap()), Some(vec![a, a1]));
		assert!(cache.read("another key").is_none(), "another key misses");
		let _ = std::fs::remove_dir_all(dir);
	}

	#[test]
	fn a_damaged_file_is_a_miss() {
		let dir = scratch("damaged");
		let cache = SheetCache::at(&dir);
		let frame = TexelRect { x: 9, y: 9, w: 4, h: 2 };
		cache.write("key", &[(frame, Region { x: 0, y: 0, w: 16, h: 8 }, &[image(16, 8, 3)][..])]).expect("write");
		let file = cache.file_for("key");
		let bytes = std::fs::read(&file).expect("the file");
		std::fs::write(&file, &bytes[..bytes.len() - 10]).expect("truncate");
		assert!(cache.read("key").is_none(), "its frame runs past the end");
		std::fs::write(&file, b"not a sheet").expect("overwrite");
		assert!(cache.read("key").is_none());
		let _ = std::fs::remove_dir_all(dir);
	}
}
