//! A minimal PNG writer (std only) for screenshots of the offscreen UI.
//!
//! The image data goes into uncompressed ("stored") deflate blocks: larger files,
//! but no compression code. Fine for test artifacts.

use std::io::{self, Write};

/// Write an RGBA image (`width * height * 4` bytes, rows top to bottom) as a PNG.
pub fn write_rgba(out: &mut impl Write, width: u32, height: u32, rgba: &[u8]) -> io::Result<()> {
	assert_eq!(
		rgba.len(),
		width as usize * height as usize * 4,
		"rgba size"
	);
	out.write_all(b"\x89PNG\r\n\x1a\n")?;

	let mut header = Vec::with_capacity(13);
	header.extend_from_slice(&width.to_be_bytes());
	header.extend_from_slice(&height.to_be_bytes());
	// 8 bits per channel, color type 6 (RGBA), deflate, adaptive filtering, no interlace.
	header.extend_from_slice(&[8, 6, 0, 0, 0]);
	chunk(out, b"IHDR", &header)?;

	// Each row is prefixed with filter type 0 (none).
	let row = width as usize * 4;
	let mut raw = Vec::with_capacity((row + 1) * height as usize);
	for line in rgba.chunks_exact(row) {
		raw.push(0);
		raw.extend_from_slice(line);
	}
	chunk(out, b"IDAT", &zlib_stored(&raw))?;
	chunk(out, b"IEND", &[])
}

/// Write a BGRA image (Chromium's paint format) as an RGBA PNG.
pub fn write_bgra(out: &mut impl Write, width: u32, height: u32, bgra: &[u8]) -> io::Result<()> {
	let mut rgba = bgra.to_vec();
	for pixel in rgba.chunks_exact_mut(4) {
		pixel.swap(0, 2);
	}
	write_rgba(out, width, height, &rgba)
}

fn chunk(out: &mut impl Write, kind: &[u8; 4], data: &[u8]) -> io::Result<()> {
	out.write_all(&(data.len() as u32).to_be_bytes())?;
	out.write_all(kind)?;
	out.write_all(data)?;
	let mut crc = Crc32::new();
	crc.update(kind);
	crc.update(data);
	out.write_all(&crc.finish().to_be_bytes())
}

/// A zlib stream of `data` in stored deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
	const MAX_BLOCK: usize = 0xFFFF;
	let mut out = Vec::with_capacity(data.len() + data.len() / MAX_BLOCK * 5 + 11);
	out.extend_from_slice(&[0x78, 0x01]);
	let mut blocks = data.chunks(MAX_BLOCK).peekable();
	if blocks.peek().is_none() {
		out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
	}
	while let Some(block) = blocks.next() {
		let last = blocks.peek().is_none();
		let len = block.len() as u16;
		out.push(last as u8);
		out.extend_from_slice(&len.to_le_bytes());
		out.extend_from_slice(&(!len).to_le_bytes());
		out.extend_from_slice(block);
	}
	out.extend_from_slice(&adler32(data).to_be_bytes());
	out
}

fn adler32(data: &[u8]) -> u32 {
	let (mut a, mut b) = (1u32, 0u32);
	for chunk in data.chunks(5552) {
		for &byte in chunk {
			a += byte as u32;
			b += a;
		}
		a %= 65521;
		b %= 65521;
	}
	b << 16 | a
}

struct Crc32 {
	table: [u32; 256],
	value: u32,
}

impl Crc32 {
	fn new() -> Self {
		let mut table = [0u32; 256];
		for (n, entry) in table.iter_mut().enumerate() {
			let mut c = n as u32;
			for _ in 0..8 {
				c = if c & 1 != 0 {
					0xEDB8_8320 ^ (c >> 1)
				} else {
					c >> 1
				};
			}
			*entry = c;
		}
		Crc32 {
			table,
			value: 0xFFFF_FFFF,
		}
	}

	fn update(&mut self, data: &[u8]) {
		for &byte in data {
			self.value =
				self.table[((self.value ^ byte as u32) & 0xFF) as usize] ^ (self.value >> 8);
		}
	}

	fn finish(&self) -> u32 {
		self.value ^ 0xFFFF_FFFF
	}
}

#[cfg(test)]
mod tests {
	use super::{Crc32, adler32, write_rgba};

	#[test]
	fn checksums_match_known_values() {
		let mut crc = Crc32::new();
		crc.update(b"123456789");
		assert_eq!(crc.finish(), 0xCBF4_3926);
		assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
	}

	#[test]
	fn writes_a_well_formed_file() {
		let mut out = Vec::new();
		write_rgba(&mut out, 2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
		assert!(out.starts_with(b"\x89PNG"));
		assert!(out.ends_with(&[0xAE, 0x42, 0x60, 0x82]), "IEND crc");
	}
}
