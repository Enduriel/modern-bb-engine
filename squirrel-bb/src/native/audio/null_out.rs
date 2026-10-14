//! The silent output the harness plays into. Sounds still take their time: each plays
//! for its file's length (see [`length`](super::length)), at its speed, while it isn't
//! paused, so effects end, the ambience schedules its next sound and music moves to its
//! next track when the original's would. A file that doesn't exist doesn't start.

use std::collections::HashMap;

use super::length::sound_length_ms;
use super::out::{AudioOut, Voice};
use crate::module::FileReader;

struct NullVoice {
	length_ms: u32,
	/// Milliseconds of the file played.
	played: f64,
	speed: f32,
	paused: bool,
	stopped: bool,
}

pub struct NullOut {
	read: FileReader,
	/// Each file's length, `None` for a file that can't be read.
	lengths: HashMap<String, Option<u32>>,
	voices: HashMap<Voice, NullVoice>,
	next: Voice,
	now: Option<u32>,
}

impl NullOut {
	/// Reading files (for their lengths) through `read`.
	pub fn new(read: FileReader) -> NullOut {
		NullOut {
			read,
			lengths: HashMap::new(),
			voices: HashMap::new(),
			next: 1,
			now: None,
		}
	}

	fn length(&mut self, file: &str) -> Option<u32> {
		if let Some(known) = self.lengths.get(file) {
			return *known;
		}
		let length = (self.read)(file).and_then(|bytes| sound_length_ms(&bytes));
		self.lengths.insert(file.to_string(), length);
		length
	}

	fn voice(&mut self, voice: Voice) -> Option<&mut NullVoice> {
		self.voices.get_mut(&voice)
	}
}

impl AudioOut for NullOut {
	fn start(&mut self, file: &str, _at: Option<(f32, f32)>) -> Option<Voice> {
		let length_ms = self.length(file)?;
		let voice = self.next;
		self.next += 1;
		self.voices.insert(
			voice,
			NullVoice {
				length_ms,
				played: 0.0,
				speed: 1.0,
				paused: true,
				stopped: false,
			},
		);
		Some(voice)
	}

	fn set_volume(&mut self, _voice: Voice, _volume: f32) {}

	fn set_pan(&mut self, _voice: Voice, _pan: f32) {}

	fn set_speed(&mut self, voice: Voice, speed: f32) {
		if let Some(v) = self.voice(voice) {
			v.speed = speed;
		}
	}

	fn set_paused(&mut self, voice: Voice, paused: bool) {
		if let Some(v) = self.voice(voice) {
			v.paused = paused;
		}
	}

	fn stop(&mut self, voice: Voice) {
		if let Some(v) = self.voice(voice) {
			v.stopped = true;
		}
	}

	fn release(&mut self, voice: Voice) {
		self.voices.remove(&voice);
	}

	fn position_ms(&self, voice: Voice) -> u32 {
		self.voices.get(&voice).map_or(0, |v| v.played as u32)
	}

	fn stopped(&self, voice: Voice) -> bool {
		self.voices.get(&voice).is_none_or(|v| v.stopped)
	}

	fn length_ms(&mut self, file: &str) -> u32 {
		self.length(file).unwrap_or(0)
	}

	fn set_listener(&mut self, _at: (f32, f32)) {}

	fn update(&mut self, now_ms: u32) {
		let dt = self.now.map_or(0, |then| now_ms.saturating_sub(then));
		self.now = Some(now_ms);
		for v in self.voices.values_mut().filter(|v| !v.paused && !v.stopped) {
			v.played += f64::from(dt) * f64::from(v.speed);
			if v.played >= f64::from(v.length_ms) {
				v.played = f64::from(v.length_ms);
				v.stopped = true;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use std::rc::Rc;

	use super::*;

	fn wav_of_ms(ms: u32) -> Vec<u8> {
		let frames = 1000 * ms / 1000;
		let mut b = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0".to_vec();
		b.extend_from_slice(&1000u32.to_le_bytes());
		b.extend_from_slice(&2000u32.to_le_bytes());
		b.extend_from_slice(&2u16.to_le_bytes());
		b.extend_from_slice(&16u16.to_le_bytes());
		b.extend_from_slice(b"data");
		b.extend_from_slice(&(frames * 2).to_le_bytes());
		b.resize(b.len() + frames as usize * 2, 0);
		b
	}

	#[test]
	fn a_sound_plays_for_its_length_while_unpaused() {
		let mut out = NullOut::new(Rc::new(|path: &str| (path == "a.wav").then(|| wav_of_ms(500))));
		assert_eq!(out.start("missing.wav", None), None);
		let v = out.start("a.wav", None).expect("starts");
		out.update(0);
		out.update(1000);
		assert!(!out.stopped(v), "paused: it hasn't played");
		out.set_paused(v, false);
		out.set_speed(v, 2.0);
		out.update(1200);
		assert_eq!(out.position_ms(v), 400);
		out.update(1300);
		assert!(out.stopped(v), "500 ms played");
	}
}
