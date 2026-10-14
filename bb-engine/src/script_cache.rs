//! The on-disk script cache: scripts in our VM's bytecode format, so a warm start skips
//! decrypting and transcoding the game's `.cnut` files.
//!
//! - **Location:** a per-user cache directory, outside the game install (see
//!   [`ScriptCache::from_env`]). Under it, one subdirectory per engine fingerprint, so a
//!   new transcoder never reads an old one's output and stale generations can be
//!   deleted wholesale.
//! - **Entries:** one file per entry, named by a hash of its [`CacheKey`] and read only
//!   when that script loads. Scripts that never load cost nothing.
//! - **Validation:** each file starts with a header carrying the full key and a payload
//!   checksum. A missing, corrupt, truncated or colliding file is a miss, and the caller
//!   takes the normal path. Writes go to a temporary file that is renamed into place, so
//!   a reader never sees a partial file.
//! - **Failures:** filesystem errors are never fatal. The cache is an optimisation.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Bump to invalidate every cache generation (e.g. when the file format changes).
/// 2: compiled sources are named `<path>.nut`, as the original names them.
const FORMAT_VERSION: u32 = 2;
const MAGIC: &[u8; 4] = b"BBSC";

/// What an entry was derived from. Two loads with equal keys must produce identical
/// bytes.
pub struct CacheKey<'a> {
	/// What was done to the input: `"cnut"` (decrypt + transcode) or `"nut"` (compile).
	pub kind: &'a str,
	/// The archive the entry came from.
	pub archive: &'a Path,
	/// The entry's path inside the archive.
	pub entry: &'a str,
	/// The CRC32 and uncompressed size the archive's central directory records.
	pub crc32: u32,
	pub size: u64,
	/// Anything else the output depends on: for a compile, the VM's compile context
	/// (constant table and debug-info flag); `0` for a transcode.
	pub context: u64,
}

impl CacheKey<'_> {
	fn encode(&self) -> Vec<u8> {
		let mut out = Vec::new();
		for part in [
			self.kind.as_bytes(),
			self.archive.as_os_str().as_encoded_bytes(),
			self.entry.as_bytes(),
		] {
			out.extend_from_slice(part);
			out.push(0);
		}
		out.extend_from_slice(&self.crc32.to_le_bytes());
		out.extend_from_slice(&self.size.to_le_bytes());
		out.extend_from_slice(&self.context.to_le_bytes());
		out
	}
}

/// FNV-1a, 64-bit.
fn fnv1a(bytes: &[u8]) -> u64 {
	bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
		(h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
	})
}

/// A fast payload checksum (8 bytes per step), to catch corrupt or partly written
/// files. Not cryptographic; the cache directory is the user's own.
fn checksum(bytes: &[u8]) -> u64 {
	const K: u64 = 0x9e37_79b9_7f4a_7c15;
	let mut words = bytes.chunks_exact(8);
	let mut h = (bytes.len() as u64).wrapping_mul(K);
	for word in &mut words {
		h = (h ^ u64::from_le_bytes(word.try_into().expect("8-byte chunk")))
			.wrapping_mul(K)
			.rotate_left(29);
	}
	for &b in words.remainder() {
		h = (h ^ u64::from(b)).wrapping_mul(K).rotate_left(29);
	}
	h
}

/// Identifies the code whose output the cache holds: the file format, the sources of the
/// decryption and the transcoder, and the VM (its bytecode format and compiler), so
/// changing any of them starts a new generation.
fn engine_fingerprint() -> u64 {
	let mut parts = FORMAT_VERSION.to_le_bytes().to_vec();
	parts.extend_from_slice(squirrel::VM_SOURCE_HASH.as_bytes());
	parts.extend_from_slice(include_str!("crypt.rs").as_bytes());
	parts.extend_from_slice(include_str!("transcode.rs").as_bytes());
	fnv1a(&parts)
}

/// An on-disk cache of script bytecode (see the module docs).
pub struct ScriptCache {
	dir: PathBuf,
}

impl ScriptCache {
	/// A cache rooted at `root` (the engine-fingerprint subdirectory is added).
	pub fn at(root: impl AsRef<Path>) -> ScriptCache {
		ScriptCache {
			dir: root
				.as_ref()
				.join(format!("v{:016x}", engine_fingerprint())),
		}
	}

	/// The cache configured by the environment:
	/// - `BB_SCRIPT_CACHE=off` (or `0`) disables it;
	/// - `BB_SCRIPT_CACHE=<dir>` roots it at `dir`;
	/// - otherwise it goes in `$XDG_CACHE_HOME/bb-modern-engine/scripts`, falling back to
	///   `~/.cache/bb-modern-engine/scripts`.
	///
	/// `None` if disabled or no location can be determined.
	pub fn from_env() -> Option<ScriptCache> {
		let root = match std::env::var_os("BB_SCRIPT_CACHE") {
			Some(v) if v == "off" || v == "0" => return None,
			Some(dir) if !dir.is_empty() => PathBuf::from(dir),
			_ => std::env::var_os("XDG_CACHE_HOME")
				.filter(|v| !v.is_empty())
				.map(PathBuf::from)
				.or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))?
				.join("bb-modern-engine")
				.join("scripts"),
		};
		Some(ScriptCache::at(root))
	}

	/// The directory this cache generation lives in.
	pub fn dir(&self) -> &Path {
		&self.dir
	}

	fn file_for(&self, key: &[u8]) -> PathBuf {
		self.dir.join(format!("{:016x}.bin", fnv1a(key)))
	}

	/// The cached bytes for `key`, or `None` on a miss (including a corrupt file).
	pub fn get(&self, key: &CacheKey) -> Option<Vec<u8>> {
		let key = key.encode();
		let mut data = fs::read(self.file_for(&key)).ok()?;
		let payload_start = parse_header(&data, &key)?;
		Some(data.split_off(payload_start))
	}

	/// Store `bytes` under `key`. Errors are ignored: the next load is just a miss.
	pub fn put(&self, key: &CacheKey, bytes: &[u8]) {
		let key = key.encode();
		let _ = self.write(&key, bytes);
	}

	fn write(&self, key: &[u8], bytes: &[u8]) -> std::io::Result<()> {
		fs::create_dir_all(&self.dir)?;
		// Unique per process and write, so concurrent writers never share a temp file.
		static WRITES: AtomicU64 = AtomicU64::new(0);
		let n = WRITES.fetch_add(1, Ordering::Relaxed);
		let tmp = self.dir.join(format!(".tmp-{}-{n}", std::process::id()));
		let mut file = fs::File::create(&tmp)?;
		file.write_all(MAGIC)?;
		file.write_all(&FORMAT_VERSION.to_le_bytes())?;
		file.write_all(&(key.len() as u32).to_le_bytes())?;
		file.write_all(key)?;
		file.write_all(&(bytes.len() as u64).to_le_bytes())?;
		file.write_all(&checksum(bytes).to_le_bytes())?;
		file.write_all(bytes)?;
		drop(file);
		fs::rename(&tmp, self.file_for(key)).inspect_err(|_| {
			let _ = fs::remove_file(&tmp);
		})
	}
}

/// Check a file's header against `key` and its payload against the stored length and
/// checksum; the payload's offset if everything matches.
fn parse_header(data: &[u8], key: &[u8]) -> Option<usize> {
	let mut at = 0;
	let mut take = |n: usize| {
		let slice = data.get(at..at + n)?;
		at += n;
		Some(slice)
	};
	if take(4)? != MAGIC || take(4)? != FORMAT_VERSION.to_le_bytes() {
		return None;
	}
	let key_len = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
	if take(key_len)? != key {
		return None; // a different key whose hash collided
	}
	let len = u64::from_le_bytes(take(8)?.try_into().ok()?) as usize;
	let stored = u64::from_le_bytes(take(8)?.try_into().ok()?);
	let payload = data.get(at..)?;
	(payload.len() == len && checksum(payload) == stored).then_some(at)
}

#[cfg(test)]
mod tests {
	use super::{CacheKey, ScriptCache};
	use std::path::Path;

	fn key(entry: &str, crc32: u32) -> CacheKey<'_> {
		CacheKey {
			kind: "cnut",
			archive: Path::new("/x/data_001.dat"),
			entry,
			crc32,
			size: 3,
			context: 0,
		}
	}

	fn temp_cache(name: &str) -> ScriptCache {
		let root = std::env::temp_dir().join(format!(
			"bb-script-cache-test-{name}-{}",
			std::process::id()
		));
		let _ = std::fs::remove_dir_all(&root);
		ScriptCache::at(root)
	}

	#[test]
	fn round_trips_and_misses_on_any_key_difference() {
		let cache = temp_cache("roundtrip");
		assert_eq!(cache.get(&key("a.cnut", 1)), None);
		cache.put(&key("a.cnut", 1), b"abc");
		assert_eq!(cache.get(&key("a.cnut", 1)).as_deref(), Some(&b"abc"[..]));
		assert_eq!(cache.get(&key("a.cnut", 2)), None); // changed entry
		assert_eq!(cache.get(&key("b.cnut", 1)), None);
		cache.put(&key("a.cnut", 1), b"xyz"); // overwrite
		assert_eq!(cache.get(&key("a.cnut", 1)).as_deref(), Some(&b"xyz"[..]));
		let _ = std::fs::remove_dir_all(cache.dir().parent().unwrap());
	}

	#[test]
	fn a_damaged_file_is_a_miss() {
		let cache = temp_cache("damaged");
		cache.put(&key("a.cnut", 1), b"payload");
		let file = std::fs::read_dir(cache.dir())
			.unwrap()
			.next()
			.unwrap()
			.unwrap()
			.path();
		let good = std::fs::read(&file).unwrap();
		let mut flipped = good.clone();
		*flipped.last_mut().unwrap() ^= 1; // payload byte
		for damaged in [
			flipped,
			good[..good.len() - 1].to_vec(),
			good[..10].to_vec(),
			Vec::new(),
		] {
			std::fs::write(&file, damaged).unwrap();
			assert_eq!(cache.get(&key("a.cnut", 1)), None);
		}
		let _ = std::fs::remove_dir_all(cache.dir().parent().unwrap());
	}
}
