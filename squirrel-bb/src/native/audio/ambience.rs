//! The ambience (`AmbienceMixer`; `Sound.setAmbience` `0x4adf90`, its update
//! `0x4ad7f0`; disassembled 2026-10-14): three layers (`layer` 0, 1, anything else the
//! third), each a list of sounds of which one plays at a time, picked at random, as a
//! one-shot on the ambience channel:
//! - at its entry's volume times the layer's, times `0.6–1.0` at random unless the
//!   entry's `RandomVolume` is false; panned at random over `[-1, 1)` unless its `Pan`
//!   is false; at its `Pitch`, times `0.9–1.1` at random if `RandomPitch`;
//! - the next sound comes after the longer of this one's length (plus a second for
//!   sounds up to 8 s) and the layer's minimum delay, on the real clock, so pausing
//!   doesn't stop it; with the channel silent it checks again after the minimum delay;
//! - setting a layer starts it at once; its sounds already playing play on (nothing
//!   fades), and its old timer lapses (each setting takes a new ticket). An empty list
//!   empties the layer, which then stops scheduling.
//!
//! The draws are the original's own audio stream's, apart from every game stream.

use super::manager::{AMBIENCE, SoundManager, Timer};

/// One of a layer's sounds (`Const.SoundAmbience` entries).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
	/// `"sounds/" + File`.
	pub file: String,
	pub volume: f32,
	pub pitch: f32,
	pub pan: bool,
	pub random_volume: bool,
	pub random_pitch: bool,
}

impl Entry {
	/// An entry with the original's defaults for the flags a script leaves out.
	pub fn new(file: String, volume: f32, pitch: f32) -> Entry {
		Entry {
			file,
			volume,
			pitch,
			pan: true,
			random_volume: true,
			random_pitch: false,
		}
	}
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Ambience {
	pub entries: Vec<Entry>,
	pub volume: f32,
	/// Milliseconds.
	pub min_delay: u32,
	pub ticket: u32,
	/// When its last sound started (real ms).
	pub last_start: u32,
}

/// Which layer a script's `layer` names.
pub(crate) fn layer_index(layer: i64) -> usize {
	match layer {
		0 => 0,
		1 => 1,
		_ => 2,
	}
}

/// The pitch factor's spread: `0x3e4cccd0` (just over 0.2).
const PITCH_SPREAD: f32 = 0.200_000_05;

impl SoundManager {
	/// `Sound.setAmbience(layer, entries, volume, minDelay)`.
	pub fn set_ambience(&mut self, layer: usize, entries: Vec<Entry>, volume: f32, min_delay: u32) {
		let a = &mut self.ambience[layer];
		if entries.is_empty() {
			a.entries.clear();
			return;
		}
		// The original loads a layer's sounds here (`0x4adf90`); the output reads
		// them ahead.
		for e in &entries {
			self.out.preload(&e.file);
		}
		let a = &mut self.ambience[layer];
		a.entries = entries;
		a.ticket = a.ticket.wrapping_add(1);
		a.volume = volume;
		a.min_delay = min_delay;
		self.ambience_next(layer);
	}

	/// `Sound.stopAmbience()`: empty every layer and stop the ambience's sounds now.
	pub fn stop_ambience(&mut self) {
		for a in &mut self.ambience {
			a.entries.clear();
		}
		self.stop_channel(AMBIENCE);
	}

	/// The layer's next sound, if it's time (`0x4ad7f0`).
	pub(crate) fn ambience_next(&mut self, layer: usize) {
		let (n, min_delay, last_start, ticket, volume) = {
			let a = &self.ambience[layer];
			(a.entries.len(), a.min_delay, a.last_start, a.ticket, a.volume)
		};
		if n == 0 {
			return;
		}
		if !(self.audible(AMBIENCE) > 0.0 && self.now >= last_start.wrapping_add(min_delay)) {
			self.schedule(self.now.wrapping_add(min_delay), Timer::Ambience { layer, ticket });
			return;
		}
		let index = if n > 1 { self.rng.range(0, n as i64 - 1) as usize } else { 0 };
		let e = self.ambience[layer].entries[index].clone();
		if let Some(id) = self.start(&e.file, None, AMBIENCE, true) {
			let random = if e.random_volume { self.rng.unit() * 0.4 + 0.6 } else { 1.0 };
			self.set_volume(id, (e.volume * volume) * random);
			if e.pan {
				let pan = (self.rng.unit() * 2.0 + 0.0) - 1.0;
				if let Some(i) = self.instance(id) {
					self.out.set_pan(i.voice, pan);
				}
			}
			let pitch = if e.random_pitch { e.pitch * (self.rng.unit() * PITCH_SPREAD + 0.9) } else { e.pitch };
			self.set_speed(id, pitch);
			self.play(id);
		}
		let length = self.out.length_ms(&e.file);
		self.ambience[layer].last_start = self.now;
		let delay = length.wrapping_add(if length > 8000 { 0 } else { 1000 }).max(min_delay);
		self.schedule(self.now.wrapping_add(delay), Timer::Ambience { layer, ticket });
	}
}

#[cfg(test)]
mod tests {
	use super::super::manager::tests::manager;
	use super::*;

	fn rain() -> Vec<Entry> {
		vec![Entry {
			pan: false,
			random_volume: false,
			..Entry::new("sounds/atmosphere/rain_01.wav".into(), 3.0, 1.0)
		}]
	}

	#[test]
	fn a_layer_plays_its_sounds_one_after_another() {
		let (mut m, log, _) = manager(2000);
		m.update(10_000);
		m.set_ambience(0, rain(), 0.5, 500);
		assert_eq!(log.borrow().started.len(), 1, "at once");
		assert_eq!(log.borrow().volume.last(), Some(&(1, 0.4 * 1.0 * (3.0 * 0.5))), "ambience 0.4 × 3.0 × 0.5");
		m.update(12_999);
		assert_eq!(log.borrow().started.len(), 1);
		m.update(13_000);
		assert_eq!(log.borrow().started.len(), 2, "after its 2 s and a second");
	}

	#[test]
	fn a_new_setting_drops_the_old_schedule_and_an_empty_one_ends_it() {
		let (mut m, log, _) = manager(2000);
		m.update(10_000);
		m.set_ambience(1, rain(), 1.0, 0);
		m.update(10_100);
		m.set_ambience(1, rain(), 1.0, 0);
		assert_eq!(log.borrow().started.len(), 2);
		m.update(13_000);
		assert_eq!(log.borrow().started.len(), 2, "the first timer lapsed");
		m.update(13_100);
		assert_eq!(log.borrow().started.len(), 3);
		m.set_ambience(1, Vec::new(), 1.0, 0);
		m.update(20_000);
		assert_eq!(log.borrow().started.len(), 3, "emptied: no more");
	}

	#[test]
	fn stopping_the_ambience_stops_its_sounds() {
		let (mut m, log, _) = manager(60_000);
		m.update(10_000);
		m.set_ambience(0, rain(), 1.0, 0);
		m.stop_ambience();
		assert_eq!(log.borrow().stopped, [1]);
		m.update(80_000);
		assert_eq!(log.borrow().started.len(), 1);
	}
}
