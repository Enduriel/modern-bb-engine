//! Sprites' animations: the frames a brush plays, as the original's sprite plays them
//! (its update `0x52c1c4`, `setBrush` `0x52bf50`, and the sprite's `play`, `stop` and
//! `finish`; disassembled 2026-10-14). Few vanilla brushes have more than one frame: a
//! world party's fight (`battle_animation`), the battle's swamp bubbles, rain splashes
//! and the world's birds.
//!
//! - Setting a different brush starts it on its first frame at the current virtual
//!   millisecond (`Time`'s virtual seconds × 1000), so a paused game holds it and the
//!   game's speed hastens it.
//! - It moves on by the whole frames elapsed since its last step, the brush's frame
//!   time each, keeping the remainder. The original steps a sprite when it draws it;
//!   here every live sprite steps each frame, so a window changes nothing.
//! - Past the last frame, a brush without the loop flag (`0x10`) stops on its last
//!   frame. A looping one starts over; a brush with a pause range (the bubbles, 800 to
//!   9000 ms) first holds its first frame for a pause, drawn when the brush was set and
//!   again at each start-over for the next one.
//! - A brush with flag `0x1000` (the bubbles) gets a new scale at each start-over, 0.5
//!   to 1.0 in steps of 0.01.
//! - `play()` starts it again, `stop()` holds the frame it shows, and `finish()` holds
//!   its last frame.
//!
//! The original draws the pauses from the C runtime's `rand()`, which 0-arg
//! `Math.rand` reads too, so drawing swamp bubbles shifts what scripts get from it.
//! Here each sprite has its own stream, seeded from its brush and when it was set,
//! which nothing else reads (see `docs/differences.md`).

use bb_assets::BrushSprite;
use squirrel::{Number, SQVM, SqStrongRef, SqTable};

use crate::native::Sprites;

/// The loop flag of a brush's flags.
const LOOPS: u32 = 0x10;
/// A new scale at each start-over.
const RESCALES: u32 = 0x1000;

/// What an animation needs of its brush.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Timing {
	frames: u32,
	frame_ms: u32,
	looping: bool,
	/// The pause before each start-over, drawn in `lo..hi` ms (none when equal).
	pause: (u32, u32),
	rescales: bool,
}

impl Timing {
	pub(crate) fn of(sprite: &BrushSprite) -> Timing {
		Timing {
			frames: sprite.frames.len() as u32,
			frame_ms: sprite.frame_time_ms,
			looping: sprite.flags & LOOPS != 0,
			pause: (u32::from(sprite.unknown_pause[0]), u32::from(sprite.unknown_pause[1])),
			rescales: sprite.flags & RESCALES != 0,
		}
	}

	/// Has frames to play (a frame time of 0 would divide by zero in the original).
	fn animated(&self) -> bool {
		self.frames > 1 && self.frame_ms > 0
	}
}

/// A sprite's place in its brush's frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Anim {
	/// `None` until it starts: when its brush is set, or at its first step.
	run: Option<Run>,
}

#[derive(Clone, Debug, PartialEq)]
struct Run {
	timing: Option<Timing>,
	frame: u32,
	/// The virtual millisecond of its last step.
	last: u32,
	/// Start-overs left: -1 for ever, 0 to stop at the end (the original's `+0x74`).
	loops: i32,
	/// The pause before the next start-over, in ms (`+0x78`).
	pause: u32,
	stopped: bool,
	/// Its own random stream (SplitMix64).
	rng: u64,
}

impl Run {
	fn next(&mut self) -> u64 {
		self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
		let mut z = self.rng;
		z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
		z ^ (z >> 31)
	}

	/// A pause in `lo..hi`, as `rand() % (hi - lo) + lo` with the runtime's 15-bit
	/// `rand()`; none for an empty range.
	fn draw_pause(&mut self, (lo, hi): (u32, u32)) -> u32 {
		if hi == lo {
			return 0;
		}
		(self.next() as u32 & 0x7fff) % hi.wrapping_sub(lo) + lo
	}
}

impl Anim {
	/// A sprite made showing `brush`, at virtual millisecond `now`.
	pub(crate) fn on(sprites: &Sprites, brush: &str, now: u32) -> Anim {
		let mut a = Anim::default();
		a.start(timing_of(sprites, brush), brush, now);
		a
	}

	/// Start on brush `name` (`timing` `None`: an unknown brush) at virtual millisecond
	/// `now`.
	pub(crate) fn start(&mut self, timing: Option<Timing>, name: &str, now: u32) {
		let mut seed = 0xcbf2_9ce4_8422_2325_u64;
		for b in name.bytes().chain(now.to_le_bytes()) {
			seed = (seed ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
		}
		let mut run = Run {
			timing,
			frame: 0,
			last: now,
			loops: if timing.is_some_and(|t| t.looping) { -1 } else { 0 },
			pause: 0,
			stopped: !timing.is_some_and(|t| t.animated()),
			rng: seed,
		};
		if let Some(t) = timing {
			run.pause = run.draw_pause(t.pause);
		}
		self.run = Some(run);
	}

	/// The frame it shows.
	pub(crate) fn frame(&self) -> u32 {
		self.run.as_ref().map_or(0, |r| r.frame)
	}

	/// `Finished`: its frames have stopped (`stop`, `finish`, or the end of its run)
	/// and `play` hasn't started them again (the sprite's flag bit 2: set at
	/// `0x504b30`, `0x52c100` and `0x504b17`, cleared at `0x52c0e0`).
	pub(crate) fn finished(&self) -> bool {
		self.run.as_ref().is_some_and(|r| r.stopped)
	}

	/// Whether a step may still change it (it needn't be visited otherwise).
	pub(crate) fn running(&self) -> bool {
		self.run.as_ref().is_none_or(|r| !r.stopped)
	}

	/// Brush `new` set on a sprite showing `old`: a different, known brush starts it
	/// again (`0x52c08b`).
	pub(crate) fn rebrush(&mut self, sprites: &Sprites, old: &str, new: &str, now: u32) {
		if new != old {
			if let Some(timing) = timing_of(sprites, new) {
				self.start(Some(timing), new, now);
			}
		}
	}

	/// Step a sprite showing `brush` to virtual millisecond `now` (see [`Anim::step`]).
	pub(crate) fn step_on(&mut self, sprites: &Sprites, brush: &str, now: u32) -> Option<f32> {
		if !self.running() {
			return None;
		}
		self.step(now, || timing_of(sprites, brush))
	}

	/// Step to virtual millisecond `now`; `timing` starts it if it hasn't started.
	/// Returns the new scale of a start-over that sets one.
	pub(crate) fn step(&mut self, now: u32, timing: impl FnOnce() -> Option<Timing>) -> Option<f32> {
		if self.run.is_none() {
			self.start(timing(), "", now);
		}
		let r = self.run.as_mut()?;
		let t = r.timing?;
		if r.stopped || r.last.wrapping_add(t.frame_ms) > now {
			return None;
		}
		if !t.animated() {
			r.stopped = true;
			return None;
		}
		let elapsed = now.wrapping_sub(r.last);
		let rem = elapsed % t.frame_ms;
		let mut hold = 0;
		r.frame += elapsed / t.frame_ms;
		if r.frame < t.frames {
			r.last = now.wrapping_sub(rem);
			return None;
		}
		if r.loops != 0 && r.loops != -1 {
			let left = r.loops - (elapsed / (t.frame_ms * t.frames)) as i32 - 1;
			r.loops = left.max(0);
		}
		if r.loops == 0 {
			r.stopped = true;
			r.frame = t.frames - 1;
			r.last = now.wrapping_sub(rem);
			return None;
		}
		r.frame %= t.frames;
		if r.pause != 0 {
			// Hold the first frame for the pause (in whole frames), and draw the next.
			let over = (r.frame as i32 * t.frame_ms as i32 - r.pause as i32) / t.frame_ms as i32;
			r.pause = r.draw_pause(t.pause);
			if over < 0 {
				hold = (t.frame_ms as i32 * -over) as u32;
				r.frame = 0;
			}
		}
		let scale = t.rescales.then(|| (r.next() % 51) as f32 / 100.0 + 0.5);
		r.last = now.wrapping_sub(rem).wrapping_add(hold);
		scale
	}

	/// `play()`: from the first frame again, at virtual millisecond `now`.
	pub(crate) fn play(&mut self, now: u32) {
		if let Some(r) = self.run.as_mut() {
			r.stopped = false;
			r.last = now;
			r.frame = 0;
		}
	}

	/// `stop()`: hold the frame it shows.
	pub(crate) fn stop(&mut self) {
		if let Some(r) = self.run.as_mut() {
			r.stopped = true;
		}
	}

	/// `finish()`: hold its last frame.
	pub(crate) fn finish(&mut self) {
		if let Some(r) = self.run.as_mut() {
			r.stopped = true;
			if let Some(t) = r.timing {
				r.frame = t.frames.saturating_sub(1);
			}
		}
	}
}

/// Brush `name`'s timing (`None` for an unknown brush).
fn timing_of(sprites: &Sprites, name: &str) -> Option<Timing> {
	sprites.with(|r| r.get(name).map(|(_, s)| Timing::of(s)))
}

/// The virtual millisecond (`Time`'s `+0x18`: virtual seconds × 1000, truncated).
pub(crate) fn virtual_ms(vm: &SQVM) -> u32 {
	SqTable::root(vm)
		.get::<SqStrongRef>("Time")
		.and_then(|t| t.call_method_ret::<Number, _>(vm, "getVirtualTimeF", ()).ok().flatten())
		.map_or(0, |n| (n.0 * 1000.0) as u32)
}

#[cfg(test)]
mod tests {
	use super::*;
	use bb_assets::BrushFrame;

	fn brush(frames: usize, frame_ms: u32, flags: u32, pause: [u16; 2]) -> BrushSprite {
		BrushSprite {
			hash: 0,
			name: "b".into(),
			material: 0,
			width: 1,
			height: 1,
			offset_x: 0,
			offset_y: 0,
			depth_bias: 0,
			frame_time_ms: frame_ms,
			unknown_pause: pause,
			rotation_speed: 0.0,
			flags,
			unknown_offset: [0.0; 2],
			color: [255; 4],
			unknown_scale: 2.0,
			frames: (0..frames)
				.map(|i| BrushFrame {
					name: i.to_string(),
					uv: [0.0; 4],
					rect: [0.0; 4],
				})
				.collect(),
			trailer: [0; 3],
		}
	}

	fn started(b: &BrushSprite, now: u32) -> Anim {
		let mut a = Anim::default();
		a.start(Some(Timing::of(b)), "b", now);
		a
	}

	fn frames_at(a: &mut Anim, times: &[u32]) -> Vec<u32> {
		times
			.iter()
			.map(|&t| {
				a.step(t, || None);
				a.frame()
			})
			.collect()
	}

	#[test]
	fn a_looping_brush_plays_on_virtual_milliseconds() {
		let b = brush(9, 100, LOOPS | 3, [0, 0]);
		let mut a = started(&b, 1000);
		assert_eq!(frames_at(&mut a, &[1000, 1099, 1100, 1250, 1899, 1900, 2050]), [0, 0, 1, 2, 8, 0, 1]);
		// Stepped rarely, it lands where steady steps would.
		let mut rare = started(&b, 1000);
		assert_eq!(frames_at(&mut rare, &[5555]), [(4555 / 100 % 9) as u32]);
		// A paused clock holds it.
		assert_eq!(frames_at(&mut a, &[2050, 2050]), [1, 1]);
	}

	#[test]
	fn a_brush_without_the_loop_flag_stops_on_its_last_frame() {
		let b = brush(4, 100, 0x403, [0, 0]);
		let mut a = started(&b, 0);
		assert_eq!(frames_at(&mut a, &[150, 399, 400, 5000]), [1, 3, 3, 3]);
		assert!(!a.running());
		a.play(6000);
		assert_eq!(frames_at(&mut a, &[6000, 6100]), [0, 1], "play() starts it again");
	}

	#[test]
	fn a_pause_holds_the_first_frame_between_loops() {
		let b = brush(6, 100, LOOPS | RESCALES | 3, [800, 9000]);
		let mut a = started(&b, 0);
		let first = a.run.as_ref().unwrap().pause;
		assert!((800..9000).contains(&first), "{first}");
		// Through the first loop, then the start-over holds frame 0 for the pause.
		for t in (0..600).step_by(50) {
			a.step(t, || None);
		}
		assert_eq!(a.frame(), 5);
		let scale = a.step(600, || None).expect("a new scale at the start-over");
		assert!((0.5..=1.0).contains(&scale), "{scale}");
		let held = first / 100 * 100;
		assert_eq!(frames_at(&mut a, &[600 + held - 1, 600 + held + 100]), [0, 1]);
	}

	#[test]
	fn single_frames_and_unknown_brushes_stay_put() {
		let mut a = started(&brush(1, 100, LOOPS, [0, 0]), 0);
		assert_eq!(frames_at(&mut a, &[1000]), [0]);
		assert!(!a.running());
		let mut unknown = Anim::default();
		unknown.start(None, "gone", 0);
		assert_eq!(frames_at(&mut unknown, &[1000]), [0]);
	}

	#[test]
	fn stop_holds_and_finish_jumps_to_the_last_frame() {
		let b = brush(9, 100, LOOPS, [0, 0]);
		let mut a = started(&b, 0);
		a.step(250, || None);
		a.stop();
		assert_eq!(frames_at(&mut a, &[900]), [2]);
		a.finish();
		assert_eq!(a.frame(), 8);
	}

	#[test]
	fn it_starts_at_its_first_step_when_no_brush_was_set_here() {
		let b = brush(3, 200, LOOPS, [0, 0]);
		let mut a = Anim::default();
		assert_eq!(a.frame(), 0);
		a.step(1000, || Some(Timing::of(&b)));
		assert_eq!(frames_at(&mut a, &[1199, 1200]), [0, 1]);
	}
}
