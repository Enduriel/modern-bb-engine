//! A zip's central directory, read into [`Index`] entries: the archive's list of files,
//! read as the `zip` crate reads it (zip64 sizes and offsets, data prepended to the
//! archive, CP437 or UTF-8 names), without keeping its record of every entry.

use std::io::{self, Read, Seek, SeekFrom};

use super::cp437;
use super::index::{Entry, Index};

const END: u32 = 0x0605_4b50;
const END64: u32 = 0x0606_4b50;
const END64_LOCATOR: u32 = 0x0706_4b50;
const HEADER: u32 = 0x0201_4b50;
/// The end record's fixed size, and how far before the file's end it can start (a
/// comment of up to 65,535 bytes follows it).
const END_SIZE: u64 = 22;
const END_SEARCH: u64 = END_SIZE + 0xffff;

fn bad(what: &str) -> io::Error {
	io::Error::new(io::ErrorKind::InvalidData, format!("not a zip archive: {what}"))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
	u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
	u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], at: usize) -> u64 {
	u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Where the directory is: its offset in the file, its entry count, and what the archive's
/// own offsets are relative to (non-zero when data is prepended, as in a self-extractor).
struct Directory {
	start: u64,
	entries: u64,
	base: u64,
}

fn find_directory(file: &mut (impl Read + Seek)) -> io::Result<Directory> {
	let len = file.seek(SeekFrom::End(0))?;
	let from = len.saturating_sub(END_SEARCH);
	file.seek(SeekFrom::Start(from))?;
	let mut tail = Vec::new();
	file.read_to_end(&mut tail)?;
	let at = (0..tail.len().saturating_sub(END_SIZE as usize - 1))
		.rev()
		.find(|&i| u32_at(&tail, i) == END)
		.ok_or_else(|| bad("no end of central directory"))?;
	let end_pos = from + at as u64;
	let end = &tail[at..];
	let (mut entries, mut size, mut offset) = (u64::from(u16_at(end, 10)), u64::from(u32_at(end, 12)), u64::from(u32_at(end, 16)));
	let mut dir_end = end_pos;
	// A zip64 locator right before the end record points at the zip64 end record: right
	// before the locator, or where the locator says (relative to the archive's start).
	if end_pos >= 20 + 56 {
		let mut locator = [0u8; 20];
		file.seek(SeekFrom::Start(end_pos - 20))?;
		file.read_exact(&mut locator)?;
		if u32_at(&locator, 0) == END64_LOCATOR {
			let mut record = [0u8; 56];
			let mut found = None;
			for pos in [end_pos - 20 - 56, u64_at(&locator, 8)] {
				file.seek(SeekFrom::Start(pos))?;
				if file.read_exact(&mut record).is_ok() && u32_at(&record, 0) == END64 {
					found = Some(pos);
					break;
				}
			}
			let pos = found.ok_or_else(|| bad("no zip64 end record"))?;
			entries = u64_at(&record, 32);
			size = u64_at(&record, 40);
			offset = u64_at(&record, 48);
			dir_end = pos;
		}
	}
	let base = dir_end.checked_sub(size).and_then(|x| x.checked_sub(offset)).ok_or_else(|| bad("its directory doesn't fit"))?;
	Ok(Directory {
		start: base + offset,
		entries,
		base,
	})
}

/// Read the central directory of the zip in `file` into an [`Index`].
pub(super) fn read(file: &mut (impl Read + Seek)) -> io::Result<Index> {
	let dir = find_directory(file)?;
	file.seek(SeekFrom::Start(dir.start))?;
	let mut reader = io::BufReader::with_capacity(64 << 10, file);
	let mut index = Index::builder();
	let mut fixed = [0u8; 46];
	let mut name = Vec::new();
	let mut extra = Vec::new();
	for _ in 0..dir.entries {
		reader.read_exact(&mut fixed)?;
		if u32_at(&fixed, 0) != HEADER {
			return Err(bad("a broken directory entry"));
		}
		let flags = u16_at(&fixed, 8);
		let method = u16_at(&fixed, 10);
		let modified = u32::from(u16_at(&fixed, 14)) << 16 | u32::from(u16_at(&fixed, 12));
		let crc32 = u32_at(&fixed, 16);
		let mut compressed = u64::from(u32_at(&fixed, 20));
		let mut size = u64::from(u32_at(&fixed, 24));
		let (name_len, extra_len, comment_len) = (u16_at(&fixed, 28), u16_at(&fixed, 30), u16_at(&fixed, 32));
		let mut header = u64::from(u32_at(&fixed, 42));
		name.resize(usize::from(name_len), 0);
		reader.read_exact(&mut name)?;
		extra.resize(usize::from(extra_len), 0);
		reader.read_exact(&mut extra)?;
		reader.seek_relative(i64::from(comment_len))?;
		// The zip64 field holds, in order, those of the sizes and the offset that are
		// 0xffffffff in the record.
		let mut at = 0;
		while at + 4 <= extra.len() {
			let (id, len) = (u16_at(&extra, at), usize::from(u16_at(&extra, at + 2)));
			let body = &extra[at + 4..(at + 4 + len).min(extra.len())];
			if id == 0x0001 {
				let mut field = body.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap()));
				for value in [&mut size, &mut compressed, &mut header] {
					if *value == 0xffff_ffff {
						if let Some(v) = field.next() {
							*value = v;
						}
					}
				}
			}
			at += 4 + len;
		}
		let text = if flags & (1 << 11) != 0 {
			String::from_utf8_lossy(&name).into_owned()
		} else {
			cp437::decode(&name)
		};
		// Zips made on Windows can separate with backslashes; the original reads those
		// as paths (mods packed that way load there).
		let text = text.replace('\\', "/");
		index.add(
			&text,
			Entry {
				name: (0, 0),
				flags,
				method,
				crc32,
				compressed,
				size,
				header: dir.base + header,
				modified,
			},
		);
	}
	Ok(index.build())
}
