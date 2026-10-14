//! The game's sound, as the original's sound manager runs it (the original's
//! library is irrKlang 1.6, disassembled 2026-10-14):
//! - [`manager`]: the channel volumes, the playing sounds, their fades and the
//!   real-time timers;
//! - [`ambience`] and [`music`]: the two mixers the scripts drive;
//! - [`natives`]: `Sound`, `Music` and `Settings`' sound volumes;
//! - [`AudioOut`]: where the sounds go. The harness's [`NullOut`] is silent but keeps
//!   time (sounds last their files' [`length`]); the window plays through kira.
//!
//! None of it reaches the game's state or its random streams: the original draws its
//! audio's randomness from a stream of its own, and so does this. The listener is the
//! camera's centre, rounded, which `World.update` and `Tactical.update` set (`0x543f20`).

mod ambience;
mod length;
mod manager;
mod music;
mod natives;
mod null_out;
mod out;

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef, SqTable};

pub use length::{sound_length_ms, sound_length_ms_in};
pub use null_out::NullOut;
pub use out::{AudioOut, Voice};

use self::manager::SoundManager;
use crate::native::EngineRng;

/// A sound the game is playing (or holds paused).
#[derive(Clone, Debug, PartialEq)]
pub struct Playing {
	pub file: String,
	/// `Const.Sound.Channel`: 0 music, 1 effects, 2 ambience.
	pub channel: usize,
	/// Its own volume (before the channel's and the master's).
	pub volume: f32,
	pub paused: bool,
}

/// The sound service an [`Engine`](crate::Engine) holds.
#[derive(Clone)]
pub struct Audio(Rc<RefCell<SoundManager>>);

impl Audio {
	/// Playing into `out`, with its own random stream `rng`.
	pub fn new(out: Box<dyn AudioOut>, rng: EngineRng) -> Audio {
		Audio(Rc::new(RefCell::new(SoundManager::new(out, rng))))
	}

	/// Play into `out` from now on (sounds already playing are dropped).
	pub fn set_out(&self, out: Box<dyn AudioOut>) {
		let mut m = self.0.borrow_mut();
		let rng = m.rng.clone();
		let (master, channels) = (m.master, m.channels);
		*m = SoundManager::new(out, rng);
		m.master = master;
		m.channels = channels;
	}

	/// Register the natives. Call after `Settings`, `World` and `Tactical` exist:
	/// `listener` says where the camera of the scene being updated is.
	pub fn install(&self, vm: &mut SQVM, world_listener: impl Fn() -> (f32, f32) + 'static, battle_listener: impl Fn() -> (f32, f32) + 'static) {
		natives::install(vm, &self.0);
		self.listen_on(vm, "World", world_listener);
		self.listen_on(vm, "Tactical", battle_listener);
	}

	/// After `table.update()` runs, the listener moves to the camera's rounded centre.
	fn listen_on(&self, vm: &mut SQVM, table: &str, camera: impl Fn() -> (f32, f32) + 'static) {
		let inner = SqTable::root(vm).get::<SqTable>(table).and_then(|t| t.get::<SqStrongRef>("update"));
		let m = self.0.clone();
		let update = vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
			if let Some(inner) = &inner {
				inner.call(vm, ())?;
			}
			let (x, y) = camera();
			let at = (x.round() as i32 as f32, y.round() as i32 as f32);
			m.borrow_mut().out.set_listener(at);
			Ok(())
		});
		if let Some(mut t) = SqTable::root(vm).get::<SqTable>(table) {
			t.set("update", update);
		}
	}

	/// A frame passed: the real clock reads `now_ms`.
	pub fn update(&self, now_ms: u32) {
		self.0.borrow_mut().update(now_ms);
	}

	/// The game discarded a scene (see [`AudioOut::discard_unused`]).
	pub fn discard_unused(&self) {
		self.0.borrow_mut().out.discard_unused();
	}

	/// How the output has been doing since the last call (see [`AudioOut::status`]).
	pub fn output_status(&self) -> Option<String> {
		self.0.borrow_mut().out.status()
	}

	/// The sounds the game holds, oldest first.
	pub fn playing(&self) -> Vec<Playing> {
		self.0
			.borrow()
			.instances()
			.iter()
			.filter(|i| !i.finished)
			.map(|i| Playing {
				file: i.file.clone(),
				channel: i.channel,
				volume: i.vol,
				paused: i.paused,
			})
			.collect()
	}

	/// Whether the music is playing (`Music.isPlaying`).
	pub fn music_playing(&self) -> bool {
		self.0.borrow().music_playing()
	}
}
