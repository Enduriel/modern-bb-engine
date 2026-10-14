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
//!   where its pixels are), then each region's pixels as a PNG.
//! - **Validation:** a missing, foreign or truncated file is a miss; writes go to a
//!   temporary file renamed into place. Failures are never fatal: it's an optimisation.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::frames::TexelRect;
use crate::{Image, decode_png};

/// Bump when the file format changes: each version has its own folder.
const VERSION: u32 = 1;
const MAGIC: &[u8; 8] = b"BBSHEET\0";

/// Our sheets' cache folder.
#[derive(Clone, Debug)]
pub struct SheetCache {
	dir: PathBuf,
}

/// One frame in a cached sheet: its rectangle in the atlas, its region, and where the
/// region's pixels are in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetEntry {
	pub frame: TexelRect,
	pub region: TexelRect,
	offset: u64,
	len: u64,
}

/// A cached sheet's index.
#[derive(Clone, Debug)]
pub struct Sheet {
	file: PathBuf,
	pub entries: Vec<SheetEntry>,
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

	fn file_for(&self, key: &str) -> PathBuf {
		self.dir.join(format!("{:016x}.sheet", fnv1a(key.as_bytes())))
	}

	/// The sheet cached for `key`, or `None` (a miss).
	pub fn read(&self, key: &str) -> Option<Sheet> {
		let file = self.file_for(key);
		let mut f = File::open(&file).ok()?;
		let length = f.metadata().ok()?.len();
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
			let mut rect = || -> Option<TexelRect> {
				Some(TexelRect {
					x: read_u32(&mut f)?,
					y: read_u32(&mut f)?,
					w: read_u32(&mut f)?,
					h: read_u32(&mut f)?,
				})
			};
			let (frame, region) = (rect()?, rect()?);
			let (offset, len) = (read_u64(&mut f)?, read_u64(&mut f)?);
			if offset.checked_add(len)? > length {
				return None;
			}
			entries.push(SheetEntry { frame, region, offset, len });
		}
		Some(Sheet { file, entries })
	}

	/// Keep the sheet of `frames` (each frame, its region, the region's pixels) for `key`.
	pub fn write(&self, key: &str, frames: &[(TexelRect, TexelRect, &Image)]) -> io::Result<()> {
		std::fs::create_dir_all(&self.dir)?;
		let blobs: Vec<Vec<u8>> = frames.iter().map(|(_, _, image)| encode(image)).collect::<io::Result<_>>()?;
		let mut head = Vec::new();
		head.extend_from_slice(MAGIC);
		head.extend_from_slice(&VERSION.to_le_bytes());
		head.extend_from_slice(&(key.len() as u32).to_le_bytes());
		head.extend_from_slice(key.as_bytes());
		head.extend_from_slice(&(frames.len() as u32).to_le_bytes());
		let index_len = frames.len() as u64 * (8 * 4 + 16);
		let mut offset = head.len() as u64 + index_len;
		for ((frame, region, _), blob) in frames.iter().zip(&blobs) {
			for rect in [frame, region] {
				for v in [rect.x, rect.y, rect.w, rect.h] {
					head.extend_from_slice(&v.to_le_bytes());
				}
			}
			head.extend_from_slice(&offset.to_le_bytes());
			head.extend_from_slice(&(blob.len() as u64).to_le_bytes());
			offset += blob.len() as u64;
		}
		let file = self.file_for(key);
		let temp = file.with_extension(format!("tmp{}", std::process::id()));
		{
			let mut f = io::BufWriter::new(File::create(&temp)?);
			f.write_all(&head)?;
			for blob in &blobs {
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
		self.entries.iter().find(|e| e.frame == *frame)
	}

	/// `entry`'s region, read and decoded, or `None` if the file no longer has it.
	pub fn read(&self, entry: &SheetEntry) -> Option<Image> {
		let mut f = File::open(&self.file).ok()?;
		f.seek(SeekFrom::Start(entry.offset)).ok()?;
		let mut bytes = vec![0; entry.len as usize];
		f.read_exact(&mut bytes).ok()?;
		let image = decode_png(&bytes).ok()?;
		(image.width == entry.region.w && image.height == entry.region.h).then_some(image)
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

fn read_u32(f: &mut File) -> Option<u32> {
	let mut b = [0u8; 4];
	f.read_exact(&mut b).ok()?;
	Some(u32::from_le_bytes(b))
}

fn read_u64(f: &mut File) -> Option<u64> {
	let mut b = [0u8; 8];
	f.read_exact(&mut b).ok()?;
	Some(u64::from_le_bytes(b))
}

/// A length-prefixed byte string, no longer than the file.
fn read_bytes(f: &mut File, limit: u64) -> Option<Vec<u8>> {
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
		let region_a = TexelRect { x: 0, y: 0, w: 16, h: 8 };
		let frame_b = TexelRect { x: 40, y: 8, w: 8, h: 4 };
		let region_b = TexelRect { x: 32, y: 0, w: 24, h: 16 };
		assert!(cache.read("key").is_none(), "nothing yet");
		cache.write("key", &[(frame_a, region_a, &a), (frame_b, region_b, &b)]).expect("write");
		let sheet = cache.read("key").expect("cached");
		assert_eq!(sheet.entries.len(), 2);
		let entry = sheet.entry(&frame_b).expect("frame b");
		assert_eq!(entry.region, region_b);
		assert_eq!(sheet.read(entry), Some(b));
		assert_eq!(sheet.read(sheet.entry(&frame_a).unwrap()), Some(a));
		assert!(cache.read("another key").is_none(), "another key misses");
		let _ = std::fs::remove_dir_all(dir);
	}

	#[test]
	fn a_damaged_file_is_a_miss() {
		let dir = scratch("damaged");
		let cache = SheetCache::at(&dir);
		let frame = TexelRect { x: 9, y: 9, w: 4, h: 2 };
		cache.write("key", &[(frame, TexelRect { x: 0, y: 0, w: 16, h: 8 }, &image(16, 8, 3))]).expect("write");
		let file = cache.file_for("key");
		let bytes = std::fs::read(&file).expect("the file");
		std::fs::write(&file, &bytes[..bytes.len() - 10]).expect("truncate");
		assert!(cache.read("key").is_none(), "its frame runs past the end");
		std::fs::write(&file, b"not a sheet").expect("overwrite");
		assert!(cache.read("key").is_none());
		let _ = std::fs::remove_dir_all(dir);
	}
}
