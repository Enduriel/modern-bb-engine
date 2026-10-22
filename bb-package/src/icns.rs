//! macOS: the bundle's icon (`Contents/Resources/bb.icns`), written from the game's
//! own `.ico` (`bb-app/assets/bb.ico`, read with bb-app's reader), so no tool and no
//! crate is needed.
//!
//! An `.icns` file is `icns` and its length, then its elements, each a type, its length
//! and its data (lengths big-endian, counting the 8-byte headers). The icon's sizes
//! (16, 32 and 48 pixels) are the classic elements, which macOS still draws: each
//! size's RGB channels, run-length encoded one after another (`is32`, `il32`, `ih32`),
//! and its alpha as a mask (`s8mk`, `l8mk`, `h8mk`).

#[path = "../../bb-app/src/icon/ico.rs"]
#[allow(dead_code)]
mod ico;

/// The game's icon file.
const ICO: &[u8] = include_bytes!("../../bb-app/assets/bb.ico");

/// The bundle's icon file's name, as `CFBundleIconFile` names it.
pub const FILE: &str = "bb.icns";

/// The `.icns` file of the game's icon.
pub fn game_icon() -> Result<Vec<u8>, String> {
	let mut elements = Vec::new();
	for image in ico::images(ICO)? {
		let (rgb, mask): (&[u8; 4], &[u8; 4]) = match image.size {
			16 => (b"is32", b"s8mk"),
			32 => (b"il32", b"l8mk"),
			48 => (b"ih32", b"h8mk"),
			size => return Err(format!("no classic .icns element for {size}x{size}")),
		};
		let rgba = image.rgba()?;
		let mut channels = Vec::new();
		for channel in 0..3 {
			let plane: Vec<u8> = rgba.chunks_exact(4).map(|p| p[channel]).collect();
			channels.extend(pack_bits(&plane));
		}
		let alpha: Vec<u8> = rgba.chunks_exact(4).map(|p| p[3]).collect();
		elements.push((*rgb, channels));
		elements.push((*mask, alpha));
	}
	let total = 8 + elements.iter().map(|(_, data)| 8 + data.len()).sum::<usize>();
	let mut out = Vec::with_capacity(total);
	out.extend_from_slice(b"icns");
	out.extend_from_slice(&(total as u32).to_be_bytes());
	for (kind, data) in elements {
		out.extend_from_slice(&kind);
		out.extend_from_slice(&(8 + data.len() as u32).to_be_bytes());
		out.extend_from_slice(&data);
	}
	Ok(out)
}

/// `bytes` run-length encoded as the classic elements are: a byte under 0x80 is followed
/// by that many plus one bytes as they are; one from 0x80 by a byte repeated that many
/// minus 125 times (3 to 130).
fn pack_bits(bytes: &[u8]) -> Vec<u8> {
	let mut out = Vec::new();
	let mut literal: Vec<u8> = Vec::new();
	let flush = |literal: &mut Vec<u8>, out: &mut Vec<u8>| {
		for chunk in literal.chunks(128) {
			out.push(chunk.len() as u8 - 1);
			out.extend_from_slice(chunk);
		}
		literal.clear();
	};
	let mut i = 0;
	while i < bytes.len() {
		let run = bytes[i..].iter().take(130).take_while(|&&b| b == bytes[i]).count();
		if run >= 3 {
			flush(&mut literal, &mut out);
			out.push((run + 125) as u8);
			out.push(bytes[i]);
			i += run;
		} else {
			literal.push(bytes[i]);
			i += 1;
		}
	}
	flush(&mut literal, &mut out);
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Undo [`pack_bits`] for `len` bytes, as macOS reads an element.
	fn unpack(data: &[u8], len: usize) -> (Vec<u8>, usize) {
		let (mut out, mut at) = (Vec::new(), 0);
		while out.len() < len {
			let control = data[at] as usize;
			if control < 0x80 {
				out.extend_from_slice(&data[at + 1..at + 2 + control]);
				at += 2 + control;
			} else {
				out.extend(std::iter::repeat_n(data[at + 1], control - 125));
				at += 2;
			}
		}
		assert_eq!(out.len(), len, "a run past the channel's end");
		(out, at)
	}

	#[test]
	fn runs_and_literals_go_round() {
		let mut bytes = vec![7; 300];
		bytes.extend((0..=255u8).cycle().take(400));
		bytes.extend([1, 1, 2, 2, 2, 3]);
		let packed = pack_bits(&bytes);
		assert_eq!(unpack(&packed, bytes.len()), (bytes.clone(), packed.len()));
		assert!(packed.len() < 400 + 40, "the runs are packed");
	}

	#[test]
	fn the_icon_has_each_size_and_its_mask() {
		let file = game_icon().unwrap();
		assert_eq!(&file[..4], b"icns");
		assert_eq!(u32::from_be_bytes(file[4..8].try_into().unwrap()) as usize, file.len());
		let images = ico::images(ICO).unwrap();
		let mut at = 8;
		let mut kinds = Vec::new();
		for image in &images {
			let rgba = image.rgba().unwrap();
			let pixels = (image.size * image.size) as usize;
			// The colour element: three channels, each packed on its own.
			let len = u32::from_be_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
			kinds.push(String::from_utf8_lossy(&file[at..at + 4]).into_owned());
			let mut data = &file[at + 8..at + len];
			for channel in 0..3 {
				let (plane, used) = unpack(data, pixels);
				assert!(plane.iter().zip(rgba.chunks_exact(4)).all(|(&c, p)| c == p[channel]));
				data = &data[used..];
			}
			assert!(data.is_empty());
			at += len;
			// The mask: the alpha as it is.
			let len = u32::from_be_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
			kinds.push(String::from_utf8_lossy(&file[at..at + 4]).into_owned());
			assert_eq!(len, 8 + pixels);
			assert!(file[at + 8..at + len].iter().zip(rgba.chunks_exact(4)).all(|(&a, p)| a == p[3]));
			at += len;
		}
		assert_eq!(at, file.len());
		assert_eq!(kinds, ["is32", "s8mk", "il32", "l8mk", "ih32", "h8mk"]);
	}
}
