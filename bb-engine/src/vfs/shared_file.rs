//! The file an opened archive reads through: positional reads on one shared handle, so
//! the archive can be cloned (each clone with its own position and read-ahead) while
//! every clone shares the archive's parsed index.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

/// How much a small read reads ahead: the `zip` crate parses headers in many small reads,
/// each a syscall without it (opening vanilla's archives took ~120 ms unbuffered, ~7 ms
/// buffered).
const READ_AHEAD: usize = 8 << 10;

pub(crate) struct SharedFile {
	file: Arc<File>,
	len: u64,
	pos: u64,
	/// What was read ahead, from `ahead_at` in the file.
	ahead: Vec<u8>,
	ahead_at: u64,
}

impl SharedFile {
	pub(crate) fn new(file: File) -> io::Result<SharedFile> {
		let len = file.metadata()?.len();
		Ok(SharedFile {
			file: Arc::new(file),
			len,
			pos: 0,
			ahead: Vec::new(),
			ahead_at: 0,
		})
	}
}

impl Clone for SharedFile {
	/// The same file, at the same position, with a read-ahead of its own.
	fn clone(&self) -> SharedFile {
		SharedFile {
			file: self.file.clone(),
			len: self.len,
			pos: self.pos,
			ahead: Vec::new(),
			ahead_at: 0,
		}
	}
}

impl Read for SharedFile {
	fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
		if out.is_empty() || self.pos >= self.len {
			return Ok(0);
		}
		let ahead_end = self.ahead_at + self.ahead.len() as u64;
		if !(self.ahead_at..ahead_end).contains(&self.pos) {
			if out.len() >= READ_AHEAD {
				// A large read goes straight to the file.
				let n = read_at(&self.file, out, self.pos)?;
				self.pos += n as u64;
				return Ok(n);
			}
			self.ahead.resize(READ_AHEAD, 0);
			let n = read_at(&self.file, &mut self.ahead, self.pos)?;
			self.ahead.truncate(n);
			self.ahead_at = self.pos;
		}
		let from = (self.pos - self.ahead_at) as usize;
		let n = out.len().min(self.ahead.len() - from);
		out[..n].copy_from_slice(&self.ahead[from..from + n]);
		self.pos += n as u64;
		Ok(n)
	}
}

impl Seek for SharedFile {
	fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
		let pos = match to {
			SeekFrom::Start(n) => Some(n),
			SeekFrom::End(d) => self.len.checked_add_signed(d),
			SeekFrom::Current(d) => self.pos.checked_add_signed(d),
		};
		self.pos = pos.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))?;
		Ok(self.pos)
	}
}

#[cfg(unix)]
fn read_at(file: &File, out: &mut [u8], at: u64) -> io::Result<usize> {
	std::os::unix::fs::FileExt::read_at(file, out, at)
}

#[cfg(windows)]
fn read_at(file: &File, out: &mut [u8], at: u64) -> io::Result<usize> {
	// Moves the handle's own position too, which no reader here relies on.
	std::os::windows::fs::FileExt::seek_read(file, out, at)
}

#[cfg(test)]
mod tests {
	use std::io::{Read, Seek, SeekFrom, Write};

	use super::*;

	#[test]
	fn clones_read_on_their_own() {
		let path = std::env::temp_dir().join(format!("bb-shared-file-{}", std::process::id()));
		let bytes: Vec<u8> = (0..40_000u32).map(|i| (i % 251) as u8).collect();
		std::fs::File::create(&path).unwrap().write_all(&bytes).unwrap();
		let mut a = SharedFile::new(File::open(&path).unwrap()).unwrap();
		let mut b = a.clone();
		let mut small = [0u8; 10];
		a.seek(SeekFrom::Start(100)).unwrap();
		a.read_exact(&mut small).unwrap();
		assert_eq!(small, bytes[100..110]);
		b.seek(SeekFrom::End(-10)).unwrap();
		b.read_exact(&mut small).unwrap();
		assert_eq!(small, bytes[39_990..]);
		// `a` reads on from where it was, through its read-ahead and past it.
		let mut big = vec![0u8; 20_000];
		a.read_exact(&mut big).unwrap();
		assert_eq!(big, bytes[110..20_110]);
		let mut rest = Vec::new();
		a.read_to_end(&mut rest).unwrap();
		assert_eq!(rest, bytes[20_110..]);
		assert!(b.seek(SeekFrom::Current(-50_000)).is_err());
		let _ = std::fs::remove_file(&path);
	}
}
