//! How long a sound file plays, from its headers, as irrKlang's `getPlayLength`
//! answers it (milliseconds at normal speed): the ambience schedules its next sound
//! by it, and music preloads the next track 3 s before a track's end.
//!
//! The game ships PCM `.wav` sounds and Ogg Vorbis music; anything else is `None`.

/// The length of the sound in `bytes`, in milliseconds, or `None` if it isn't a
/// readable PCM wav or Ogg Vorbis file.
pub fn sound_length_ms(bytes: &[u8]) -> Option<u32> {
	sound_length_ms_in(bytes, bytes, bytes.len() as u64)
}

/// [`sound_length_ms`] from a file's start and its end, the file `len` bytes long: what a
/// sound read in place needs, not its whole file. The start holds a wav's chunks up to its
/// `data` (`fmt` comes first) or the Ogg Vorbis identification header; the end the last
/// Ogg page (64 KiB of either is plenty: an Ogg page is at most 65,307 bytes).
pub fn sound_length_ms_in(head: &[u8], tail: &[u8], len: u64) -> Option<u32> {
	if head.starts_with(b"RIFF") {
		wav_length_ms(head, len)
	} else if head.starts_with(b"OggS") {
		ogg_length_ms(head, tail)
	} else {
		None
	}
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
	Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
	Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Frames in the `data` chunk over the `fmt` chunk's sample rate, from the file's first
/// bytes `b`, the whole file `len` bytes long.
fn wav_length_ms(b: &[u8], len: u64) -> Option<u32> {
	if b.get(8..12)? != b"WAVE" {
		return None;
	}
	let (mut rate, mut block_align, mut data) = (None, None, None);
	let mut at = 12;
	while at + 8 <= b.len() {
		let id = &b[at..at + 4];
		let size = u32_at(b, at + 4)? as usize;
		let body = at + 8;
		match id {
			b"fmt " => {
				rate = u32_at(b, body + 4);
				block_align = u16_at(b, body + 12);
			}
			// A truncated last chunk counts what's there.
			b"data" => data = Some(size.min(usize::try_from(len).unwrap_or(usize::MAX).saturating_sub(body))),
			_ => {}
		}
		// Chunks are padded to an even size.
		at = body + size + (size & 1);
	}
	let (rate, block_align, data) = (rate?, block_align?, data?);
	if rate == 0 || block_align == 0 {
		return None;
	}
	let frames = (data / usize::from(block_align)) as u64;
	Some((frames * 1000 / u64::from(rate)) as u32)
}

/// The last page's granule position (the stream's length in samples, from the file's end
/// `tail`) over the Vorbis identification header's sample rate (from its start `head`).
fn ogg_length_ms(head: &[u8], tail: &[u8]) -> Option<u32> {
	// The identification packet: type 1, "vorbis", version, channels, rate.
	let id = head.windows(7).position(|w| w == b"\x01vorbis")?;
	let rate = u32_at(head, id + 12)?;
	let last = tail.windows(4).rposition(|w| w == b"OggS")?;
	let granule = u64::from_le_bytes(tail.get(last + 6..last + 14)?.try_into().ok()?);
	if rate == 0 || granule == u64::MAX {
		return None;
	}
	Some((granule * 1000 / u64::from(rate)) as u32)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn wav(rate: u32, channels: u16, frames: u32) -> Vec<u8> {
		let block = channels * 2;
		let data = frames * u32::from(block);
		let mut b = Vec::new();
		b.extend_from_slice(b"RIFF");
		b.extend_from_slice(&(36 + data).to_le_bytes());
		b.extend_from_slice(b"WAVEfmt ");
		b.extend_from_slice(&16u32.to_le_bytes());
		b.extend_from_slice(&1u16.to_le_bytes());
		b.extend_from_slice(&channels.to_le_bytes());
		b.extend_from_slice(&rate.to_le_bytes());
		b.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
		b.extend_from_slice(&block.to_le_bytes());
		b.extend_from_slice(&16u16.to_le_bytes());
		b.extend_from_slice(b"data");
		b.extend_from_slice(&data.to_le_bytes());
		b.resize(b.len() + data as usize, 0);
		b
	}

	#[test]
	fn a_wavs_length_is_its_frames_over_its_rate() {
		assert_eq!(sound_length_ms(&wav(44_100, 2, 44_100 * 3 / 2)), Some(1500));
		assert_eq!(sound_length_ms(&wav(48_000, 1, 480)), Some(10));
		assert_eq!(sound_length_ms(b"nothing"), None);
	}

	#[test]
	fn a_wavs_length_from_its_start() {
		let file = wav(44_100, 2, 44_100 * 3);
		assert_eq!(sound_length_ms_in(&file[..64], &file[file.len() - 64..], file.len() as u64), Some(3000));
	}
}
