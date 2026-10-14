//! The game's sound through kira: an [`AudioOut`] that plays what the game's sound
//! manager says, at the gains the original's library and driver give (see
//! [`gains`](crate::gains)).
//!
//! A sound the game starts paused isn't handed to kira until it first unpauses, so
//! nothing leaks out before; after that, pauses pause it in kira. Positional sounds
//! take new gains whenever the listener moves.

use std::collections::HashMap;
use std::sync::Arc;

use bb_assets::FileSource;
use kira::backend::{Backend, DefaultBackend};
use kira::sound::FromFileError;
use kira::sound::static_sound::StaticSoundHandle;
use kira::sound::streaming::StreamingSoundHandle;
use kira::sound::PlaybackState;
use kira::{AudioManager, AudioManagerSettings, Decibels, Panning, PlaybackRate, Tween};
use squirrel_bb::{AudioOut, Voice};

use crate::bank::{Sound, SoundBank};
use crate::gains::{self, Listener};

/// kira's silence: anything quieter plays as nothing.
const FLOOR_DB: f32 = -60.0;

enum Handle {
	Static(StaticSoundHandle),
	Streaming(StreamingSoundHandle<FromFileError>),
}

impl Handle {
	fn set(&mut self, db: f32, panning: f32) {
		let (db, panning) = (Decibels(db.max(FLOOR_DB)), Panning(panning));
		match self {
			Handle::Static(h) => {
				h.set_volume(db, Tween::default());
				h.set_panning(panning, Tween::default());
			}
			Handle::Streaming(h) => {
				h.set_volume(db, Tween::default());
				h.set_panning(panning, Tween::default());
			}
		}
	}

	fn set_rate(&mut self, speed: f32) {
		let rate = PlaybackRate(f64::from(speed));
		match self {
			Handle::Static(h) => h.set_playback_rate(rate, Tween::default()),
			Handle::Streaming(h) => h.set_playback_rate(rate, Tween::default()),
		}
	}

	fn pause(&mut self, paused: bool) {
		match (self, paused) {
			(Handle::Static(h), true) => h.pause(Tween::default()),
			(Handle::Static(h), false) => h.resume(Tween::default()),
			(Handle::Streaming(h), true) => h.pause(Tween::default()),
			(Handle::Streaming(h), false) => h.resume(Tween::default()),
		}
	}

	fn stop(&mut self) {
		match self {
			Handle::Static(h) => h.stop(Tween::default()),
			Handle::Streaming(h) => h.stop(Tween::default()),
		}
	}

	fn position_ms(&self) -> u32 {
		let seconds = match self {
			Handle::Static(h) => h.position(),
			Handle::Streaming(h) => h.position(),
		};
		(seconds * 1000.0) as u32
	}

	fn ended(&self) -> bool {
		let state = match self {
			Handle::Static(h) => h.state(),
			Handle::Streaming(h) => h.state(),
		};
		state == PlaybackState::Stopped
	}
}

struct KiraVoice {
	/// The sound, until it first plays.
	pending: Option<Sound>,
	handle: Option<Handle>,
	at: Option<(f32, f32)>,
	volume: f32,
	pan: f32,
	speed: f32,
	stopped: bool,
}

/// The game's sound on a kira backend (the device, or a mock for tests).
pub struct KiraOut<B: Backend> {
	manager: AudioManager<B>,
	bank: SoundBank,
	voices: HashMap<Voice, KiraVoice>,
	next: Voice,
	listener: Listener,
	rolloff: f32,
}

impl KiraOut<DefaultBackend> {
	/// On the system's default output device.
	pub fn device(files: Arc<dyn FileSource>, rolloff: f32) -> Result<Self, String> {
		Self::new(AudioManagerSettings::default(), files, rolloff).map_err(|e| e.to_string())
	}
}

impl<B: Backend> KiraOut<B> {
	/// Sounds read from `files`; positional ones rolling off by `rolloff` (see
	/// [`gains::rolloff_for_width`]).
	pub fn new(settings: AudioManagerSettings<B>, files: Arc<dyn FileSource>, rolloff: f32) -> Result<Self, B::Error> {
		Ok(KiraOut {
			manager: AudioManager::new(settings)?,
			bank: SoundBank::new(files),
			voices: HashMap::new(),
			next: 1,
			listener: Listener::Default,
			rolloff,
		})
	}

	/// The backend (a mock's, for tests to drive).
	pub fn backend_mut(&mut self) -> &mut B {
		self.manager.backend_mut()
	}

	/// The kira volume and panning a voice plays at now.
	fn settings(&self, v: &KiraVoice) -> (f32, f32) {
		let gains = match v.at {
			None => gains::flat(v.volume, v.pan),
			Some(at) => gains::positional(v.volume, at, self.listener, self.rolloff),
		};
		gains::kira_settings(gains)
	}

	fn apply(&mut self, voice: Voice) {
		let Some(v) = self.voices.get(&voice) else { return };
		let (db, panning) = self.settings(v);
		if let Some(h) = self.voices.get_mut(&voice).and_then(|v| v.handle.as_mut()) {
			h.set(db, panning);
		}
	}

	/// Hand a voice's sound to kira, as it is now.
	fn begin(&mut self, voice: Voice) {
		let Some(v) = self.voices.get(&voice) else { return };
		let (db, panning) = self.settings(v);
		let Some(v) = self.voices.get_mut(&voice) else { return };
		let Some(sound) = v.pending.take() else { return };
		let (db, panning, rate) = (Decibels(db.max(FLOOR_DB)), Panning(panning), PlaybackRate(f64::from(v.speed)));
		let handle = match sound {
			Sound::Static(data) => self
				.manager
				.play(data.volume(db).panning(panning).playback_rate(rate))
				.map(Handle::Static)
				.map_err(|e| e.to_string()),
			Sound::Streaming(data) => self
				.manager
				.play(data.volume(db).panning(panning).playback_rate(rate))
				.map(Handle::Streaming)
				.map_err(|e| e.to_string()),
		};
		match handle {
			Ok(h) => v.handle = Some(h),
			Err(e) => {
				eprintln!("audio: {e}");
				v.stopped = true;
			}
		}
	}
}

impl<B: Backend> AudioOut for KiraOut<B> {
	fn start(&mut self, file: &str, at: Option<(f32, f32)>) -> Option<Voice> {
		let sound = self.bank.get(file)?;
		let voice = self.next;
		self.next += 1;
		self.voices.insert(
			voice,
			KiraVoice {
				pending: Some(sound),
				handle: None,
				at,
				volume: 1.0,
				pan: 0.0,
				speed: 1.0,
				stopped: false,
			},
		);
		Some(voice)
	}

	fn set_volume(&mut self, voice: Voice, volume: f32) {
		if let Some(v) = self.voices.get_mut(&voice) {
			v.volume = volume;
		}
		self.apply(voice);
	}

	fn set_pan(&mut self, voice: Voice, pan: f32) {
		if let Some(v) = self.voices.get_mut(&voice) {
			v.pan = pan;
		}
		self.apply(voice);
	}

	fn set_speed(&mut self, voice: Voice, speed: f32) {
		let Some(v) = self.voices.get_mut(&voice) else { return };
		v.speed = speed;
		if let Some(h) = v.handle.as_mut() {
			h.set_rate(speed);
		}
	}

	fn set_paused(&mut self, voice: Voice, paused: bool) {
		let Some(v) = self.voices.get_mut(&voice) else { return };
		if v.stopped {
			return;
		}
		match v.handle.as_mut() {
			Some(h) => h.pause(paused),
			None if !paused => self.begin(voice),
			None => {}
		}
	}

	fn stop(&mut self, voice: Voice) {
		let Some(v) = self.voices.get_mut(&voice) else { return };
		v.stopped = true;
		v.pending = None;
		if let Some(h) = v.handle.as_mut() {
			h.stop();
		}
	}

	fn release(&mut self, voice: Voice) {
		if let Some(mut v) = self.voices.remove(&voice) {
			if let Some(h) = v.handle.as_mut() {
				h.stop();
			}
		}
	}

	fn position_ms(&self, voice: Voice) -> u32 {
		self.voices.get(&voice).and_then(|v| v.handle.as_ref()).map_or(0, Handle::position_ms)
	}

	fn stopped(&self, voice: Voice) -> bool {
		self.voices.get(&voice).is_none_or(|v| v.stopped || v.handle.as_ref().is_some_and(Handle::ended))
	}

	fn length_ms(&mut self, file: &str) -> u32 {
		self.bank.length_ms(file)
	}

	fn preload(&mut self, file: &str) {
		self.bank.length_ms(file);
	}

	fn set_listener(&mut self, at: (f32, f32)) {
		let listener = Listener::At(at.0, at.1);
		if self.listener == listener {
			return;
		}
		self.listener = listener;
		let positional: Vec<Voice> = self.voices.iter().filter(|(_, v)| v.at.is_some()).map(|(id, _)| *id).collect();
		for voice in positional {
			self.apply(voice);
		}
	}

	fn update(&mut self, _now_ms: u32) {}
}
