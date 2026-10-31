//! The game's sound through kira: an [`AudioOut`] that plays what the game's sound
//! manager says, at the gains the original's library and driver give (see
//! [`gains`](crate::gains)), except that positional sounds pan by their place on the
//! screen (see [`pan`](crate::pan)).
//!
//! A sound the game starts paused isn't handed to kira until it first unpauses, so
//! nothing leaks out before; after that, pauses pause it in kira. Positional sounds
//! take new gains whenever the listener moves or the zoom changes.

use std::collections::HashMap;
use std::sync::Arc;

use bb_assets::FileSource;
use kira::backend::mock::{MockBackend, MockBackendSettings};
use kira::backend::{Backend, DefaultBackend};
use kira::sound::FromFileError;
use kira::sound::static_sound::StaticSoundHandle;
use kira::sound::streaming::StreamingSoundHandle;
use kira::sound::PlaybackState;
use kira::{AudioManager, AudioManagerSettings, Decibels, Panning, PlaybackRate, Tween};
use squirrel_bb::{AudioOut, Voice};

use crate::bank::{Sound, SoundBank};
use crate::gains::{self, Listener};
use crate::pan::{self, View};

/// kira's silence: anything quieter plays as nothing.
const FLOOR_DB: f32 = -60.0;

/// The device's buffer size (about 43 ms at 48 kHz). The audio thread runs at normal
/// priority, so a buffer is the most it can fall behind before the sound gaps; the
/// device's default on PipeWire is 512 frames (10.7 ms), and stalls of 30–120 ms lost
/// every buffer they covered at 512 and 1024 frames, while 2048 absorbed stalls up to
/// about its length (measured 2026-10-14).
pub const DEVICE_BUFFER_FRAMES: u32 = 2048;

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
	view: View,
	rolloff: f32,
	/// The backend's report (only the device has one).
	status: fn(&mut AudioManager<B>) -> Option<String>,
}

impl KiraOut<DefaultBackend> {
	/// On the system's default output device, with buffers of `buffer_frames` frames
	/// (`None`: the device's default), or `BB_AUDIO_BUFFER` frames if that's set.
	pub fn device_with(files: Arc<dyn FileSource>, rolloff: f32, buffer_frames: Option<u32>) -> Result<Self, String> {
		use kira::backend::cpal::cpal;
		use kira::backend::cpal::cpal::traits::{DeviceTrait, HostTrait};
		let buffer_frames = std::env::var("BB_AUDIO_BUFFER").ok().and_then(|v| v.parse().ok()).or(buffer_frames);
		let mut settings = AudioManagerSettings::<DefaultBackend>::default();
		if let Some(frames) = buffer_frames {
			let device = cpal::default_host().default_output_device().ok_or("no default output device")?;
			let mut config = device.default_output_config().map_err(|e| e.to_string())?.config();
			config.buffer_size = cpal::BufferSize::Fixed(frames);
			settings.backend_settings.config = Some(config);
		}
		let mut out = Self::new(settings, files, rolloff).map_err(|e| e.to_string())?;
		out.status = |manager| Some(device_stats(manager).to_string());
		Ok(out)
	}

	/// On the system's default output device, with [`DEVICE_BUFFER_FRAMES`]-frame
	/// buffers, or the device's own if it won't take those.
	pub fn device(files: Arc<dyn FileSource>, rolloff: f32) -> Result<Self, String> {
		Self::device_with(files.clone(), rolloff, Some(DEVICE_BUFFER_FRAMES)).or_else(|e| {
			eprintln!("audio: {DEVICE_BUFFER_FRAMES}-frame buffers refused ({e}); using the device's");
			Self::device_with(files, rolloff, None)
		})
	}

	/// What the device's audio thread reported since the last call.
	pub fn device_stats(&mut self) -> DeviceStats {
		device_stats(&mut self.manager)
	}
}

/// The rate the silent output renders at.
const SILENT_RATE: u32 = 44_100;

impl KiraOut<MockBackend> {
	/// With no device: kira's mock backend, which renders only when [`KiraOut::run_silent`]
	/// says so, and nothing is heard. For tools that measure the sound, not play it.
	pub fn silent(files: Arc<dyn FileSource>, rolloff: f32) -> Result<Self, String> {
		let settings = AudioManagerSettings {
			backend_settings: MockBackendSettings { sample_rate: SILENT_RATE },
			..AudioManagerSettings::default()
		};
		// The mock can't fail to start.
		Self::new(settings, files, rolloff).map_err(|()| "kira's mock backend".to_string())
	}

	/// Render `ms` of sound into nothing, so voices play on and end as on a device.
	pub fn run_silent(&mut self, ms: u32) {
		// The mock renders 128 frames a call.
		for _ in 0..(SILENT_RATE * ms / 1000).div_ceil(128) {
			let backend = self.manager.backend_mut();
			backend.on_start_processing();
			backend.process();
		}
	}
}

fn device_stats(manager: &mut AudioManager<DefaultBackend>) -> DeviceStats {
	{
		let backend = manager.backend_mut();
		let mut stats = DeviceStats::default();
		let mut total = 0.0;
		while let Some(load) = backend.pop_cpu_usage() {
			stats.callbacks += 1;
			total += load;
			stats.max_load = stats.max_load.max(load);
		}
		if stats.callbacks > 0 {
			stats.mean_load = total / stats.callbacks as f32;
		}
		while let Some(e) = backend.pop_error() {
			if e.kind() == kira::backend::cpal::cpal::ErrorKind::Xrun {
				stats.xruns += 1;
			} else {
				stats.errors.push(e.to_string());
			}
		}
		stats
	}
}

impl std::fmt::Display for DeviceStats {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"{} buffers, load mean {:.2} max {:.2}, {} underruns",
			self.callbacks, self.mean_load, self.max_load, self.xruns
		)?;
		for e in &self.errors {
			write!(f, "; {e}")?;
		}
		Ok(())
	}
}

/// The device's audio thread over a stretch of time: how many buffers it filled, how
/// much of each buffer's time filling it took (1 or more is too late: a gap), and the
/// underruns the driver reported.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceStats {
	pub callbacks: usize,
	pub mean_load: f32,
	pub max_load: f32,
	pub xruns: usize,
	pub errors: Vec<String>,
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
			view: View::DEFAULT,
			rolloff,
			status: |_| None,
		})
	}

	/// The voices the output holds, those whose sound never started (still paused since
	/// `start`), and those that ended but weren't released.
	pub fn voice_use(&self) -> (usize, usize, usize) {
		let pending = self.voices.values().filter(|v| v.pending.is_some()).count();
		let ended = self.voices.values().filter(|v| v.handle.as_ref().is_some_and(Handle::ended)).count();
		(self.voices.len(), pending, ended)
	}

	/// What the sound bank holds: see [`SoundBank::memory_use`].
	pub fn bank_use(&self) -> (usize, usize, usize, usize) {
		self.bank.memory_use()
	}

	/// The backend (a mock's, for tests to drive).
	pub fn backend_mut(&mut self) -> &mut B {
		self.manager.backend_mut()
	}

	/// The kira volume and panning a voice plays at now.
	fn settings(&self, v: &KiraVoice) -> (f32, f32) {
		let gains = match v.at {
			None => gains::flat(v.volume, v.pan),
			Some(at) => pan::positional(v.volume, at, self.view, self.rolloff),
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
		self.bank.prefetch(file);
	}

	fn discard_unused(&mut self) {
		self.bank.discard_unused();
	}

	fn set_listener(&mut self, at: (f32, f32), half_width: f32) {
		let view = View {
			listener: Listener::At(at.0, at.1),
			half_width,
		};
		if self.view == view {
			return;
		}
		self.view = view;
		let positional: Vec<Voice> = self.voices.iter().filter(|(_, v)| v.at.is_some()).map(|(id, _)| *id).collect();
		for voice in positional {
			self.apply(voice);
		}
	}

	fn update(&mut self, _now_ms: u32) {
		self.bank.tick();
	}

	fn status(&mut self) -> Option<String> {
		(self.status)(&mut self.manager)
	}

	fn report_memory(&self, report: &mut squirrel_bb::memory_report::MemoryReport) {
		let (sounds, decoded, streamed, read_ahead) = self.bank_use();
		report.add("sound bank", "sounds known", sounds, 0);
		report.add("sound bank", "decoded sounds kept", 0, decoded);
		report.add("sound bank", "long sounds playing (their files)", 0, streamed);
		report.add("sound bank", "read ahead, not played yet", 0, read_ahead);
		let (voices, pending, ended) = self.voice_use();
		report.add("sound output", "voices", voices, 0);
		report.add("sound output", "voices never started", pending, 0);
		report.add("sound output", "voices ended, not released", ended, 0);
	}
}
