//! An entry an archive stores uncompressed, read in place: a long sound decoded as it
//! plays reads its file from the archive, with none of it kept in memory.

use std::io::{self, Read, Seek, SeekFrom};

use bb_assets::FileStream;

use super::shared_file::SharedFile;

/// An entry's bytes, `len` of them from `start` in the archive's file.
pub struct EntryWindow {
	file: SharedFile,
	start: u64,
	len: u64,
	pos: u64,
}

impl EntryWindow {
	pub(crate) fn new(file: SharedFile, start: u64, len: u64) -> EntryWindow {
		EntryWindow { file, start, len, pos: 0 }
	}
}

impl Read for EntryWindow {
	fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
		let left = self.len.saturating_sub(self.pos);
		let want = out.len().min(usize::try_from(left).unwrap_or(usize::MAX));
		if want == 0 {
			return Ok(0);
		}
		self.file.seek(SeekFrom::Start(self.start + self.pos))?;
		let n = self.file.read(&mut out[..want])?;
		self.pos += n as u64;
		Ok(n)
	}
}

impl Seek for EntryWindow {
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

impl FileStream for EntryWindow {
	fn len(&self) -> u64 {
		self.len
	}
}

#[cfg(test)]
mod tests {
	use std::io::{Read, Seek, SeekFrom, Write};

	use zip::CompressionMethod;
	use zip::write::SimpleFileOptions;

	use crate::vfs::DatArchive;

	#[test]
	fn a_stored_entry_reads_in_place_and_a_compressed_one_doesnt() {
		let path = std::env::temp_dir().join(format!("bb-entry-window-{}.zip", std::process::id()));
		let music: Vec<u8> = (0..50_000u32).map(|i| (i % 253) as u8).collect();
		let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
		zip.start_file("before.txt", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
		zip.write_all(b"an entry before").unwrap();
		zip.start_file("music/track.ogg", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
		zip.write_all(&music).unwrap();
		zip.start_file("packed.txt", SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).unwrap();
		zip.write_all(&[7u8; 4096]).unwrap();
		zip.finish().unwrap();

		let mut archive = DatArchive::open(&path).unwrap();
		let mut window = archive.stream("music/track.ogg").expect("stored: read in place");
		let mut all = Vec::new();
		window.read_to_end(&mut all).unwrap();
		assert_eq!(all, music);
		assert_eq!(window.seek(SeekFrom::End(-10)).unwrap(), 49_990);
		let mut tail = [0u8; 16];
		let n = window.read(&mut tail).unwrap();
		assert_eq!(&tail[..n], &music[49_990..], "stops at the entry's end");
		assert!(archive.stream("packed.txt").is_none(), "compressed: read it whole");
		assert!(archive.stream("missing.ogg").is_none());
		let _ = std::fs::remove_file(&path);
	}
}
