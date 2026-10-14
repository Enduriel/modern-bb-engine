//! Where the game's sounds go: an [`AudioOut`], shaped like the original's sound
//! library (irrKlang's `ISound`), so the game's side keeps every rule of the original
//! and the output only plays what it's told. The harness has [`NullOut`](super::NullOut)
//! (silent, but sounds still take their time); the window plays through kira
//! (`bb-audio`).
//!
//! Volumes are irrKlang's, linear and not clamped (the output applies the original's
//! conversion); pans are irrKlang's, `-1` right to `1` left; speeds scale both pitch
//! and tempo. Positions are world units, the listener the camera's centre.

/// A playing (or paused) sound.
pub type Voice = u64;

/// The sound output.
pub trait AudioOut {
	/// Start the sound in `file` paused: two-dimensional, or at the world position
	/// `at`. `None` if the file can't be read (nothing plays, as in the original).
	fn start(&mut self, file: &str, at: Option<(f32, f32)>) -> Option<Voice>;

	fn set_volume(&mut self, voice: Voice, volume: f32);

	/// A two-dimensional sound's pan.
	fn set_pan(&mut self, voice: Voice, pan: f32);

	fn set_speed(&mut self, voice: Voice, speed: f32);

	fn set_paused(&mut self, voice: Voice, paused: bool);

	/// Stop it for good; [`stopped`](Self::stopped) says so from then on.
	fn stop(&mut self, voice: Voice);

	/// Forget it (the game is done with it).
	fn release(&mut self, voice: Voice);

	/// How far it has played, in milliseconds of the file.
	fn position_ms(&self, voice: Voice) -> u32;

	/// Whether it has ended or been stopped.
	fn stopped(&self, voice: Voice) -> bool;

	/// How long `file` plays at normal speed, in milliseconds (0 if unknown).
	fn length_ms(&mut self, file: &str) -> u32;

	/// Start reading `file` ahead of its use (the next music track).
	fn preload(&mut self, _file: &str) {}

	/// The listener moved to `at` (the camera's centre, rounded).
	fn set_listener(&mut self, at: (f32, f32));

	/// The real time is `now_ms` (a frame passed).
	fn update(&mut self, now_ms: u32);

	/// The game discarded a scene (a battle ended, a campaign was left or replaced): drop
	/// what was kept only for reuse. Sounds playing play on.
	fn discard_unused(&mut self) {}

	/// How the output has been doing since the last call (a device's underruns and
	/// load), for the host to report; `None` if there's nothing to say.
	fn status(&mut self) -> Option<String> {
		None
	}
}
