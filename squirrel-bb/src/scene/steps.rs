//! A battle's animations in steps: the clock its scene is drawn at.
//!
//! The original draws a battle at 60 frames a second, every frame something new: the
//! active unit's hop, the details turning, the clouds drifting, the rain's fades. At a
//! display rate above that, or with nothing but sub-pixel moves between two frames, a
//! frame drawn at every tick of the clock shows nothing the eye could tell from the last.
//! So while a host asks for it ([`SceneSteps::set_fit`]), a battle's scene is drawn at a
//! clock that moves in steps (as the world's fog does, see `bb_render::fog_drift_time`):
//!
//! - a step is 1/60 s (the original's frame), doubled, up to [`MOST_DOUBLINGS`] times,
//!   while nothing that moves goes further than [`STEP_PIXELS`] of a screen pixel in it
//!   (the fastest motion of the frame before, see [`SceneSteps::need`]); a fade, rain or
//!   anything else changing that isn't a motion keeps it at 1/60 s;
//! - a step is a whole number of frames, as many as fit in it (at least one, the frames'
//!   length a running mean), or as many as took that long: at 180 Hz three frames, at
//!   144 Hz two, at 60 Hz every frame. Counting frames rather than reading a grid of
//!   1/60 s off the clock keeps a display at 60 Hz, whose frames come a little early or
//!   late, from now and then holding one and jumping the next;
//! - one clock for everything the scene draws, so all its animations step together;
//! - at each step the clocks are taken (real and virtual), and held until the next: what
//!   the scene computes from time (the hop, fades) is computed at the held clocks, and
//!   what the engine moves frame by frame (details turning, clouds, rain) is shown as it
//!   was at the step (see `Engine::tactical_scene`). A frame between two steps then shows
//!   the same scene, and is left undrawn.
//!
//! Only what's drawn is held: the game's state (positions, rotations, colours the
//! scripts can read) still moves every frame. Without a fit (the default: tests, tools,
//! replays), the scene is drawn at the clocks as they are.

use std::cell::Cell;

/// The finest step, in seconds: the original's frame.
pub const STEP: f64 = 1.0 / 60.0;

/// The most a step doubles: 64/60 s, past which nothing would look like it moves.
pub const MOST_DOUBLINGS: u32 = 6;

/// The most a step may move anything, in screen pixels.
pub const STEP_PIXELS: f32 = 0.25;

/// How many times [`STEP`] doubles for a motion of `speed` screen pixels a (real) second:
/// as long as the doubled step moves it no more than [`STEP_PIXELS`].
pub fn doublings(speed: f32) -> u32 {
	let mut step = STEP as f32;
	let mut k = 0;
	while k < MOST_DOUBLINGS && speed * step * 2.0 <= STEP_PIXELS {
		step *= 2.0;
		k += 1;
	}
	k
}

/// The clocks a scene is drawn at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeldClock {
	/// Real seconds.
	pub real: f64,
	/// Virtual seconds.
	pub virtual_secs: f64,
}

/// The clocks held to the steps (see the module docs).
#[derive(Debug, Default)]
pub(crate) struct SceneSteps {
	/// Frame pixels per pixel of the game's screen, while the host draws in steps.
	fit: Cell<Option<f32>>,
	/// The clocks taken at the last step, and the frames sampled since.
	held: Cell<Option<(HeldClock, u32)>>,
	/// The last frame's real clock, and the frames' mean length (seconds).
	last_real: Cell<Option<f64>>,
	frame_length: Cell<f64>,
	/// Counts the steps taken: what's held from the scene is taken again when it moves.
	generation: Cell<u64>,
	/// The fastest motion since the last sample, in world units a real second
	/// (infinite: something that needs the finest step).
	fastest: Cell<f32>,
	/// The step's doublings now.
	doublings: Cell<u32>,
	/// The last sample took the clocks.
	stepped: Cell<bool>,
}

impl SceneSteps {
	/// Draw in steps for a frame of `fit` pixels to the game screen's one, or not.
	pub fn set_fit(&self, fit: Option<f32>) {
		self.fit.set(fit);
		if fit.is_none() {
			self.held.set(None);
		}
	}

	/// Something moves `speed` world units a real second; the next step is short enough
	/// for it.
	pub fn need(&self, speed: f32) {
		if speed > self.fastest.get() {
			self.fastest.set(speed);
		}
	}

	/// Something changes that needs the finest step (a fade, the rain).
	pub fn need_finest(&self) {
		self.fastest.set(f32::INFINITY);
	}

	/// A frame's start, at the clocks `now`, where a world unit is `zoom` pixels of the
	/// game's screen: take the clocks if a step's frames have passed (or its time, or the
	/// virtual clock went back, a new campaign's). Returns whether it did.
	pub fn sample(&self, now: HeldClock, zoom: f32) -> bool {
		let fastest = self.fastest.replace(0.0);
		let length = match self.last_real.replace(Some(now.real)) {
			Some(last) if now.real > last => {
				let (mean, frame) = (self.frame_length.get(), now.real - last);
				if mean > 0.0 {
					mean * 0.9 + frame * 0.1
				} else {
					frame
				}
			}
			_ => self.frame_length.get(),
		};
		self.frame_length.set(length);
		let Some(fit) = self.fit.get() else {
			self.generation.set(self.generation.get() + 1);
			self.stepped.set(true);
			return true;
		};
		let k = doublings(fastest * fit * zoom);
		self.doublings.set(k);
		let step = STEP * f64::from(1u32 << k);
		// The frames that fit in a step, give or take a frame's jitter.
		let frames = if length > 0.0 {
			((step / length + 0.05).floor() as u32).max(1)
		} else {
			1
		};
		let stepped = match self.held.get() {
			Some((held, since)) => {
				let since = since + 1;
				let stepped = since >= frames
					|| now.real - held.real >= step
					|| now.virtual_secs < held.virtual_secs;
				if !stepped {
					self.held.set(Some((held, since)));
				}
				stepped
			}
			None => true,
		};
		if stepped {
			self.held.set(Some((now, 0)));
			self.generation.set(self.generation.get() + 1);
		}
		self.stepped.set(stepped);
		stepped
	}

	/// Whether the last [`sample`](Self::sample) took the clocks (always, off).
	pub fn stepped(&self) -> bool {
		self.stepped.get() || !self.on()
	}

	/// The clocks to draw at, the clocks being `now`.
	pub fn clock(&self, now: HeldClock) -> HeldClock {
		match (self.fit.get(), self.held.get()) {
			(Some(_), Some((held, _))) => held,
			_ => now,
		}
	}

	/// Whether the scene is drawn in steps now.
	pub fn on(&self) -> bool {
		self.fit.get().is_some()
	}

	/// The number of the step the clocks were last taken at.
	pub fn generation(&self) -> u64 {
		self.generation.get()
	}

	/// The step's doublings now (0: 1/60 s).
	pub fn doublings_now(&self) -> u32 {
		self.doublings.get()
	}
}

/// What's held from a scene until the next step: taken again at a step, or when its
/// source changes (`key`).
#[derive(Debug, Default)]
pub(crate) struct Held<K, T> {
	taken: Option<(u64, K, T)>,
}

impl<K: PartialEq, T: Clone> Held<K, T> {
	/// The value held, taken again with `take` at a new step (`generation`) or a new
	/// `key`; while off (`on` false), taken as it is now, and the one held kept.
	pub fn get(&mut self, on: bool, generation: u64, key: K, take: impl FnOnce() -> T) -> T {
		if !on {
			return take();
		}
		match &self.taken {
			Some((g, k, value)) if *g == generation && *k == key => value.clone(),
			_ => {
				let value = take();
				self.taken = Some((generation, key, value.clone()));
				value
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn at(real: f64, virtual_secs: f64) -> HeldClock {
		HeldClock { real, virtual_secs }
	}

	#[test]
	fn a_step_doubles_while_it_moves_no_more_than_a_quarter_pixel() {
		assert_eq!(doublings(f32::INFINITY), 0);
		// The hop: 7 units in 0.4 s at zoom 1, 0.29 px in 1/60 s.
		assert_eq!(doublings(17.5), 0);
		// 0.25 px in 2/60 s.
		assert_eq!(doublings(7.5), 1);
		assert_eq!(doublings(3.0), 2);
		assert_eq!(doublings(0.0), MOST_DOUBLINGS);
	}

	#[test]
	fn without_a_fit_the_clocks_are_as_they_are() {
		let s = SceneSteps::default();
		assert!(s.sample(at(1.0, 1.0), 1.0));
		assert_eq!(s.clock(at(1.01, 1.01)), at(1.01, 1.01));
	}

	#[test]
	fn a_step_is_the_frames_that_fit_in_it() {
		let steps_in = |hz: f64, frames: u32| {
			let s = SceneSteps::default();
			s.set_fit(Some(1.0));
			(0..frames)
				.filter(|&i| {
					// Frames a little early or late.
					let jitter = if i % 2 == 0 { 0.0003 } else { -0.0003 };
					s.need_finest();
					s.sample(at(100.0 + f64::from(i) / hz + jitter, 0.0), 1.0)
				})
				.count()
		};
		assert_eq!(steps_in(180.0, 180), 60);
		assert_eq!(steps_in(144.0, 144), 72);
		assert_eq!(steps_in(240.0, 240), 60);
		assert_eq!(steps_in(60.0, 60), 60, "every frame at 60 Hz");
		assert_eq!(steps_in(30.0, 30), 30);
	}

	#[test]
	fn the_clocks_hold_between_steps() {
		let s = SceneSteps::default();
		s.set_fit(Some(1.0));
		s.need_finest();
		assert!(s.sample(at(10.0, 5.0), 1.0));
		let g = s.generation();
		// 1/180 s on: the same step of 1/60 s.
		s.need_finest();
		assert!(!s.sample(at(10.0 + 1.0 / 180.0, 5.0 + 1.0 / 180.0), 1.0));
		assert_eq!(s.clock(at(10.006, 5.006)), at(10.0, 5.0));
		assert_eq!(s.generation(), g);
		s.need_finest();
		assert!(s.sample(at(10.02, 5.02), 1.0));
		assert_eq!(s.clock(at(10.021, 5.021)), at(10.02, 5.02));
	}

	#[test]
	fn slow_motion_holds_longer_and_fast_motion_steps_at_once() {
		let s = SceneSteps::default();
		s.set_fit(Some(1.0));
		assert!(s.sample(at(64.2 / 60.0, 0.0), 1.0));
		// Nothing moves: a step of 64/60 s.
		assert!(!s.sample(at(64.5 / 60.0, 0.0), 1.0));
		assert!(!s.sample(at(100.5 / 60.0, 0.0), 1.0));
		// A hop starts: a step at once.
		s.need(17.5);
		assert!(s.sample(at(101.5 / 60.0, 0.0), 1.0));
	}

	#[test]
	fn a_virtual_clock_gone_back_is_taken_at_once() {
		let s = SceneSteps::default();
		s.set_fit(Some(1.0));
		assert!(s.sample(at(1.0, 100.0), 1.0));
		assert!(s.sample(at(1.001, 0.0), 1.0));
	}

	#[test]
	fn held_values_are_taken_again_at_a_step_or_a_new_key() {
		let mut h = Held::<u64, i32>::default();
		let mut n = 0;
		let mut take = || {
			n += 1;
			n
		};
		assert_eq!(h.get(true, 1, 0, &mut take), 1);
		assert_eq!(h.get(true, 1, 0, &mut take), 1);
		assert_eq!(h.get(true, 2, 0, &mut take), 2);
		assert_eq!(h.get(true, 2, 1, &mut take), 3);
		assert_eq!(h.get(false, 2, 1, &mut take), 4);
		assert_eq!(h.get(true, 2, 1, &mut take), 3, "kept");
	}
}
