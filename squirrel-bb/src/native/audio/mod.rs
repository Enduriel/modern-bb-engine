//! The game's sound, as the original's sound manager runs it (the original's
//! library is irrKlang 1.6, disassembled 2026-10-14):
//! - [`manager`]: the channel volumes, the playing sounds and their fades, on `Time`'s
//!   real clock; the music's and the ambience's timers are events on `Time`'s list, so
//!   `Time.clearEvents()` cancels them as in the original;
//! - [`ambience`] and [`music`]: the two mixers the scripts drive;
//! - [`natives`]: `Sound`, `Music` and `Settings`' sound volumes;
//! - [`AudioOut`]: where the sounds go. The harness's [`NullOut`] is silent but keeps
//!   time (sounds last their files' [`length`]); the window plays through kira.
//!
//! None of it reaches the game's state or its random streams: the original draws its
//! audio's randomness from a stream of its own, and so does this. The listener is the
//! camera's centre, rounded, which `World.update` and `Tactical.update` set (`0x543f20`),
//! with half the view's width in world units, which the output pans against (this
//! engine's pan, not the original's: see `bb_audio::pan`).

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
use crate::native::time::TimeService;

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

impl crate::memory_report::ReportMemory for Audio {
	fn report_memory(&self, report: &mut crate::memory_report::MemoryReport) {
		self.0.borrow().out.report_memory(report);
	}
}

impl Audio {
	/// Playing into `out`, with its own random stream `rng`, on `time`'s real clock
	/// and event list.
	pub fn new(out: Box<dyn AudioOut>, rng: EngineRng, time: TimeService) -> Audio {
		Audio(Rc::new_cyclic(|me| RefCell::new(SoundManager::new(out, rng, time, me.clone()))))
	}

	/// Play into `out` from now on (sounds already playing are dropped; timers still
	/// pending find their layers empty or their tickets lapsed).
	pub fn set_out(&self, out: Box<dyn AudioOut>) {
		let mut m = self.0.borrow_mut();
		let (rng, time, me) = (m.rng.clone(), m.time.clone(), m.me.clone());
		let (master, channels) = (m.master, m.channels);
		let tickets = m.ambience.each_ref().map(|a| a.ticket);
		*m = SoundManager::new(out, rng, time, me);
		m.master = master;
		m.channels = channels;
		for (a, ticket) in m.ambience.iter_mut().zip(tickets) {
			a.ticket = ticket;
		}
	}

	/// Register the natives. Call after `Settings`, `World` and `Tactical` exist:
	/// each listener says where the camera of the scene being updated is, and how
	/// many world units its view reaches either side (half the screen's width times
	/// the zoom).
	pub fn install(&self, vm: &mut SQVM, world_listener: impl Fn() -> ((f32, f32), f32) + 'static, battle_listener: impl Fn() -> ((f32, f32), f32) + 'static) {
		natives::install(vm, &self.0);
		self.listen_on(vm, "World", world_listener);
		self.listen_on(vm, "Tactical", battle_listener);
	}

	/// After `table.update()` runs, the listener moves to the camera's rounded centre.
	fn listen_on(&self, vm: &mut SQVM, table: &str, camera: impl Fn() -> ((f32, f32), f32) + 'static) {
		let inner = SqTable::root(vm).get::<SqTable>(table).and_then(|t| t.get::<SqStrongRef>("update"));
		let m = self.0.clone();
		let update = vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
			if let Some(inner) = &inner {
				inner.call(vm, ())?;
			}
			let ((x, y), half_width) = camera();
			let at = (x.round() as i32 as f32, y.round() as i32 as f32);
			m.borrow_mut().out.set_listener(at, half_width);
			Ok(())
		});
		if let Some(mut t) = SqTable::root(vm).get::<SqTable>(table) {
			t.set("update", update);
		}
	}

	/// A frame passed (after its scheduled events): the fades and the sounds' ends.
	pub fn update(&self) {
		self.0.borrow_mut().update();
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
