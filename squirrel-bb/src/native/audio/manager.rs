//! The original's sound manager (`SoundManager`, `0x5437d0`; disassembled 2026-10-14):
//! the channel volumes, the playing sounds, their fades and the real-time timers the
//! music and ambience run on. Every frame [`update`](SoundManager::update) runs the
//! due timers, then the fades, then the sounds' ends (`0x543d70`).
//!
//! A sound (`SoundSource`, `0x542d30`/`0x542f00`) starts paused. Its volume is the
//! channel's times the master's times its own (`0x5430f0`): a sound whose volume comes
//! out exactly 0 stays paused, and resumes where it was once a volume change makes it
//! audible. `play` (`0x543190`) unpauses it unless its volume is 0.

use super::ambience::Ambience;
use super::music::Music;
use super::out::{AudioOut, Voice};
use crate::native::EngineRng;

/// The channels (`Const.Sound.Channel`).
pub(crate) const MUSIC: usize = 0;
pub(crate) const EFFECTS: usize = 1;
pub(crate) const AMBIENCE: usize = 2;

/// The game's default volumes (`0x4adad0`): master, music, effects, ambience.
const DEFAULTS: [f32; 4] = [1.0, 0.35, 0.7, 0.4];

pub(crate) type InstanceId = u64;

/// What happens when a sound ends or reaches a point (the music's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Hook {
	/// A track ended (`0x4ad720`).
	TrackEnded,
	/// 3 s before a track's end: pick and load the next (`0x4ad740`).
	PreloadNext,
}

/// A playing sound.
pub(crate) struct Instance {
	pub id: InstanceId,
	pub voice: Voice,
	pub file: String,
	pub channel: usize,
	not_started: bool,
	auto_delete: bool,
	/// Its stop event came (it ended or was stopped).
	stopped: bool,
	/// The game is done with it: it goes on the next update.
	pub finished: bool,
	/// Its own volume and speed.
	pub vol: f32,
	pub pitch: f32,
	/// The output's volume and pause, as last set.
	out_volume: f32,
	pub paused: bool,
	pub on_finished: Option<Hook>,
	pub on_position: Option<(u32, Hook)>,
}

/// A fade of one sound (`0x542b80`, `0x542c10`), on the real clock.
struct Fader {
	instance: InstanceId,
	end: u32,
	duration: u32,
	/// A fade out's starting volume; `None` fades in.
	out_from: Option<f32>,
	/// The music's, when a fade out ends.
	then: Option<FadeDone>,
	done: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FadeDone {
	/// The music faded out (`0x4ad700`).
	MusicFadedOut,
}

/// What a timer does when it's due.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Timer {
	/// The ambience layer's next sound, if its ticket still holds (`0x4ada80`).
	Ambience { layer: usize, ticket: u32 },
	/// Try to start the music again (it was silent, `0x4ad2d0`).
	Music,
}

pub(crate) struct SoundManager {
	pub out: Box<dyn AudioOut>,
	pub master: f32,
	/// The 32 channels' volumes (+0x2c; the music's is stored halved, see
	/// `Settings.getSoundVolume`).
	pub channels: [f32; 32],
	instances: Vec<Instance>,
	faders: Vec<Fader>,
	/// Due time (real ms) and what to do, in the order they were set.
	timers: Vec<(u32, Timer)>,
	pub ambience: [Ambience; 3],
	pub music: Music,
	/// The original's own random stream, apart from every game stream (`0x66eaf8`).
	pub rng: EngineRng,
	/// The real clock, in ms.
	pub now: u32,
	next_id: InstanceId,
}

impl SoundManager {
	pub fn new(out: Box<dyn AudioOut>, rng: EngineRng) -> SoundManager {
		let mut channels = [1.0; 32];
		channels[..3].copy_from_slice(&DEFAULTS[1..]);
		SoundManager {
			out,
			master: DEFAULTS[0],
			channels,
			instances: Vec::new(),
			faders: Vec::new(),
			timers: Vec::new(),
			ambience: Default::default(),
			music: Music::default(),
			rng,
			now: 0,
			next_id: 1,
		}
	}

	/// The channel's volume with the master's (what `play` checks against 0).
	pub fn audible(&self, channel: usize) -> f32 {
		self.channels[channel] * self.master
	}

	pub fn instances(&self) -> &[Instance] {
		&self.instances
	}

	pub fn instance(&self, id: InstanceId) -> Option<&Instance> {
		self.instances.iter().find(|i| i.id == id)
	}

	pub fn instance_mut(&mut self, id: InstanceId) -> Option<&mut Instance> {
		self.instances.iter_mut().find(|i| i.id == id)
	}

	/// Start `file` paused on `channel` (2D, or at `at`), `None` if it can't be read.
	pub fn start(&mut self, file: &str, at: Option<(f32, f32)>, channel: usize, auto_delete: bool) -> Option<InstanceId> {
		let voice = self.out.start(file, at)?;
		let id = self.next_id;
		self.next_id += 1;
		self.instances.push(Instance {
			id,
			voice,
			file: file.to_string(),
			channel,
			not_started: true,
			auto_delete,
			stopped: false,
			finished: false,
			vol: 1.0,
			pitch: 1.0,
			out_volume: 1.0,
			paused: true,
			on_finished: None,
			on_position: None,
		});
		Some(id)
	}

	/// Set the output's volume directly (the original's redundant `setVolume`s).
	pub fn set_out_volume(&mut self, id: InstanceId, volume: f32) {
		let Some(i) = self.instances.iter_mut().find(|i| i.id == id) else { return };
		i.out_volume = volume;
		self.out.set_volume(i.voice, volume);
	}

	/// A sound's own volume `v` (`0x5430f0`).
	pub fn set_volume(&mut self, id: InstanceId, v: f32) {
		let (master, channels) = (self.master, self.channels);
		let Some(i) = self.instances.iter_mut().find(|i| i.id == id) else { return };
		i.out_volume = (channels[i.channel] * master) * v;
		self.out.set_volume(i.voice, i.out_volume);
		i.vol = v;
		if i.not_started {
			return;
		}
		if i.paused {
			// Unpaused even if the new volume is 0, as the original.
			i.paused = false;
			self.out.set_paused(i.voice, false);
			return;
		}
		if i.out_volume == 0.0 {
			i.paused = true;
			self.out.set_paused(i.voice, true);
		}
	}

	pub fn set_speed(&mut self, id: InstanceId, speed: f32) {
		let Some(i) = self.instances.iter_mut().find(|i| i.id == id) else { return };
		i.pitch = speed;
		self.out.set_speed(i.voice, speed);
	}

	/// Start it playing (`0x543190`).
	pub fn play(&mut self, id: InstanceId) {
		let Some(i) = self.instances.iter_mut().find(|i| i.id == id) else { return };
		i.not_started = false;
		if i.out_volume != 0.0 {
			i.paused = false;
			self.out.set_paused(i.voice, false);
		}
	}

	/// The volumes changed (`0x543fb0`): every sound takes its volume and speed again.
	pub fn reapply(&mut self) {
		let all: Vec<(InstanceId, f32, f32)> = self.instances.iter().map(|i| (i.id, i.vol, i.pitch)).collect();
		for (id, vol, pitch) in all {
			self.set_volume(id, vol);
			self.set_speed(id, pitch);
		}
	}

	/// Stop every sound on `channel` now (`0x544090`).
	pub fn stop_channel(&mut self, channel: usize) {
		for i in self.instances.iter_mut().filter(|i| i.channel == channel) {
			self.out.stop(i.voice);
			if i.auto_delete {
				i.finished = true;
			}
		}
	}

	/// Fade the sound in from silence over `ms` (`0x542b80`).
	pub fn fade_in(&mut self, id: InstanceId, ms: u32) {
		self.faders.push(Fader {
			instance: id,
			end: self.now.wrapping_add(ms),
			duration: ms,
			out_from: None,
			then: None,
			done: false,
		});
		self.set_volume(id, 0.0);
	}

	/// Fade the sound out over `ms`, then stop it (`0x542c10`); `then` runs at the end.
	pub fn fade_out(&mut self, id: InstanceId, ms: u32, then: Option<FadeDone>) {
		let from = self.instance(id).map_or(1.0, |i| i.vol);
		self.faders.push(Fader {
			instance: id,
			end: self.now.wrapping_add(ms),
			duration: ms,
			out_from: Some(from),
			then,
			done: false,
		});
	}

	/// End the sound's fades without their endings (`0x543ed0`).
	pub fn cancel_fades(&mut self, id: InstanceId) {
		for f in self.faders.iter_mut().filter(|f| f.instance == id) {
			f.done = true;
			f.then = None;
		}
	}

	pub fn schedule(&mut self, due: u32, timer: Timer) {
		self.timers.push((due, timer));
	}

	/// One frame at real time `now_ms`.
	pub fn update(&mut self, now_ms: u32) {
		self.now = now_ms;
		self.out.update(now_ms);
		self.run_timers();
		self.run_faders();
		self.run_instances();
	}

	fn run_timers(&mut self) {
		while let Some(at) = self.timers.iter().position(|(due, _)| *due <= self.now) {
			let (_, timer) = self.timers.remove(at);
			match timer {
				Timer::Ambience { layer, ticket } => {
					if self.ambience[layer].ticket == ticket {
						self.ambience_next(layer);
					}
				}
				Timer::Music => self.music_start_next(false),
			}
		}
	}

	/// One step of a fade: whether it goes on.
	fn step(&mut self, f: usize) -> bool {
		let (id, end, duration, out_from) = {
			let f = &self.faders[f];
			(f.instance, f.end, f.duration, f.out_from)
		};
		let rem = (end.wrapping_sub(self.now) as i32).max(0) as u32;
		let t = duration.wrapping_sub(rem) as f32 / duration as f32;
		match out_from {
			None => self.set_volume(id, t),
			Some(from) => {
				self.set_volume(id, (1.0 - t) * from);
				if rem == 0 {
					if let Some(i) = self.instance(id) {
						self.out.stop(i.voice);
					}
					self.set_volume(id, from);
				}
			}
		}
		rem != 0
	}

	fn run_faders(&mut self) {
		let mut f = 0;
		while f < self.faders.len() {
			let id = self.faders[f].instance;
			let gone = self.instance(id).is_none_or(|i| i.finished);
			if !self.faders[f].done {
				self.faders[f].done = gone || !self.step(f);
			}
			if self.faders[f].done {
				let fader = self.faders.remove(f);
				if let Some(FadeDone::MusicFadedOut) = fader.then {
					self.music_faded_out();
				}
				continue;
			}
			f += 1;
		}
	}

	fn run_instances(&mut self) {
		let mut n = 0;
		while n < self.instances.len() {
			if self.instances[n].finished {
				let i = self.instances.remove(n);
				self.out.release(i.voice);
				continue;
			}
			let (id, voice) = (self.instances[n].id, self.instances[n].voice);
			if let Some((at, hook)) = self.instances[n].on_position {
				if self.out.position_ms(voice) >= at {
					self.instances[n].on_position = None;
					self.run_hook(hook);
				}
			}
			let Some(n_now) = self.instances.iter().position(|i| i.id == id) else { continue };
			n = n_now;
			if !self.instances[n].stopped && self.out.stopped(voice) {
				self.instances[n].stopped = true;
			}
			if self.instances[n].stopped {
				if let Some(hook) = self.instances[n].on_finished.take() {
					self.run_hook(hook);
				}
				if let Some(n_now) = self.instances.iter().position(|i| i.id == id) {
					n = n_now;
					if self.instances[n].auto_delete {
						let i = self.instances.remove(n);
						self.out.release(i.voice);
						continue;
					}
				}
			}
			n += 1;
		}
	}

	fn run_hook(&mut self, hook: Hook) {
		match hook {
			Hook::TrackEnded => self.music_track_ended(),
			Hook::PreloadNext => self.music_preload_next(),
		}
	}
}

#[cfg(test)]
pub(crate) mod tests {
	use std::cell::RefCell;
	use std::rc::Rc;

	use super::*;

	/// What an output was told, by voice.
	#[derive(Default)]
	pub struct Log {
		pub started: Vec<(Voice, String, Option<(f32, f32)>)>,
		pub volume: Vec<(Voice, f32)>,
		pub paused: Vec<(Voice, bool)>,
		pub pan: Vec<(Voice, f32)>,
		pub speed: Vec<(Voice, f32)>,
		pub stopped: Vec<Voice>,
	}

	/// An output whose sounds last `length` ms and end when told.
	pub struct TestOut {
		pub log: Rc<RefCell<Log>>,
		pub length: u32,
		pub ended: Rc<RefCell<Vec<Voice>>>,
	}

	impl AudioOut for TestOut {
		fn start(&mut self, file: &str, at: Option<(f32, f32)>) -> Option<Voice> {
			if file.contains("missing") {
				return None;
			}
			let mut log = self.log.borrow_mut();
			let voice = log.started.len() as Voice + 1;
			log.started.push((voice, file.to_string(), at));
			Some(voice)
		}
		fn set_volume(&mut self, voice: Voice, volume: f32) {
			self.log.borrow_mut().volume.push((voice, volume));
		}
		fn set_pan(&mut self, voice: Voice, pan: f32) {
			self.log.borrow_mut().pan.push((voice, pan));
		}
		fn set_speed(&mut self, voice: Voice, speed: f32) {
			self.log.borrow_mut().speed.push((voice, speed));
		}
		fn set_paused(&mut self, voice: Voice, paused: bool) {
			self.log.borrow_mut().paused.push((voice, paused));
		}
		fn stop(&mut self, voice: Voice) {
			self.log.borrow_mut().stopped.push(voice);
		}
		fn release(&mut self, _voice: Voice) {}
		fn position_ms(&self, _voice: Voice) -> u32 {
			0
		}
		fn stopped(&self, voice: Voice) -> bool {
			self.ended.borrow().contains(&voice) || self.log.borrow().stopped.contains(&voice)
		}
		fn length_ms(&mut self, _file: &str) -> u32 {
			self.length
		}
		fn set_listener(&mut self, _at: (f32, f32)) {}
		fn update(&mut self, _now_ms: u32) {}
	}

	pub fn manager(length: u32) -> (SoundManager, Rc<RefCell<Log>>, Rc<RefCell<Vec<Voice>>>) {
		let log = Rc::new(RefCell::new(Log::default()));
		let ended = Rc::new(RefCell::new(Vec::new()));
		let out = TestOut {
			log: log.clone(),
			length,
			ended: ended.clone(),
		};
		(SoundManager::new(Box::new(out), EngineRng::seeded(1)), log, ended)
	}

	#[test]
	fn a_silent_sound_stays_paused_until_it_is_audible() {
		let (mut m, log, _) = manager(1000);
		let id = m.start("a.wav", None, EFFECTS, true).unwrap();
		m.set_volume(id, 0.0);
		m.play(id);
		assert!(m.instance(id).unwrap().paused, "volume 0: still paused");
		m.set_volume(id, 0.5);
		assert!(!m.instance(id).unwrap().paused);
		assert_eq!(log.borrow().volume.last(), Some(&(1, 0.7 * 0.5)), "effects 0.7 × 0.5");
	}

	#[test]
	fn fades_run_on_the_real_clock() {
		let (mut m, log, _) = manager(10_000);
		let id = m.start("a.wav", None, EFFECTS, true).unwrap();
		m.update(1000);
		m.fade_in(id, 400);
		m.play(id);
		m.update(1100);
		assert_eq!(m.instance(id).unwrap().vol, 0.25);
		m.update(1400);
		assert_eq!(m.instance(id).unwrap().vol, 1.0);
		m.fade_out(id, 200, None);
		m.update(1500);
		assert_eq!(m.instance(id).unwrap().vol, 0.5);
		m.update(1600);
		assert_eq!(log.borrow().stopped, [1], "stopped at the end, its volume back");
		assert!(m.instance(id).is_none(), "and gone once its stop came");
	}
}
