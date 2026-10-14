//! The world clock: `World.getTime()`, `World.update()`, `World.getSpeedMult` /
//! `setSpeedMult`, and the clock part of `World.clearScene()`. Follows the
//! original's disassembly (2026-10-13), checked against probed values.
//!
//! The day is a fixed table of ten phases whose durations sum to `SecondsPerDay`
//! (105 s). Each phase carries the `TimeOfDay` and `IsDaytime` scripts read, plus the
//! world's lighting: an ambient colour and saturation (`0x4e63a0`). Every
//! `World.update()` adds
//! `speed * realDelta` (the frame's real delta in seconds) to `Time` and finds the
//! current phase. `World.setSpeedMult` also sets the `Time` service's virtual speed,
//! so `Time.getVirtualTime*` advance at the same rate. Arithmetic is `f32`, as in
//! the original.
//!
//! The lighting blends from the phase's towards the next phase's as the phase passes,
//! every update that isn't paused (`0x4e6990`). The colour blends per channel
//! (`0x4a65f0`), truncated back to a byte. While the clock is paused nothing is
//! recomputed, so a new campaign, which starts paused, keeps the scene's neutral
//! white until the game runs.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{ClassBuilder, Number, SQVM, SqStrongRef, SqTable};

use super::world_nav::cvtt_i32;

/// One phase of the day: its lighting, its duration in seconds, what scripts read
/// during it, and how well things can be seen (`World.getGlobalVisibilityMult()`).
struct Phase {
	/// Ambient colour, ARGB.
	color: u32,
	saturation: f32,
	duration: f32,
	time_of_day: i64,
	is_daytime: bool,
	visibility: f32,
}

const fn phase(
	color: u32,
	saturation: f32,
	duration: f32,
	time_of_day: i64,
	is_daytime: bool,
	visibility: f32,
) -> Phase {
	Phase {
		color,
		saturation,
		duration,
		time_of_day,
		is_daytime,
		visibility,
	}
}

/// The original's day, in order from the start of a day (`Const.World.TimeOfDay`
/// values). Dusk and Night are each split in two for lighting.
const PHASES: [Phase; 10] = [
	phase(0xffffa462, 1.0, 3.0, 0, true, 1.0),   // Dawn
	phase(0xffffb762, 1.0, 7.0, 1, true, 1.0),   // Morning
	phase(0xffffffff, 1.0, 30.0, 2, true, 1.0),  // Midday
	phase(0xffffffff, 1.0, 20.0, 3, true, 1.0),  // Afternoon
	phase(0xffd9e5f1, 1.0, 5.0, 4, true, 1.0),   // Evening
	phase(0xffeb8c6f, 1.0, 5.0, 5, true, 0.75),  // Dusk
	phase(0xffeb8c6f, 1.0, 5.0, 5, true, 0.75),  // Dusk
	phase(0xff7083a8, 0.8, 18.0, 6, false, 0.5), // Night
	phase(0xff7083a8, 0.8, 6.0, 6, false, 0.5),  // Night
	phase(0xffb0c7f4, 0.9, 6.0, 7, true, 0.75),  // EarlyMorning
];

/// `a` blended towards `b`, `keep` of the way kept at `a` (`0x4a65f0`): per channel
/// in `f32`, truncated.
fn blend_color(a: u32, b: u32, keep: f32) -> u32 {
	if a == b {
		return a;
	}
	let toward = 1.0 - keep;
	let unit = |c: u32, shift: u32| ((c >> shift) & 0xff) as i32 as f32 * (1.0f32 / 255.0);
	[0, 8, 16, 24].iter().fold(0, |out, &shift| {
		let v = (toward * unit(b, shift) + unit(a, shift) * keep) * 255.0;
		out | (((v as i32) as u32 & 0xff) << shift)
	})
}

/// The world's clock state (the original's `WorldTime`).
pub(crate) struct WorldClock {
	/// Seconds since the clock's origin.
	time: f32,
	seconds_per_day: f32,
	/// Index into [`PHASES`] as of the last update.
	phase: usize,
	/// Seconds into that phase.
	into_phase: f32,
	/// `World.getGlobalVisibilityMult()`: the phase's visibility, blended towards the
	/// next phase's as the phase passes.
	global_visibility: f32,
	/// The world's ambient colour (ARGB) and saturation, for the renderer.
	ambient: u32,
	saturation: f32,
	/// While set, `World.update` doesn't advance the clock (`IsPaused`).
	paused: bool,
	/// `World.getSpeedMult()`: the scale on real time (0 when the game is paused).
	speed: f32,
}

impl Default for WorldClock {
	fn default() -> Self {
		Self::new()
	}
}

impl WorldClock {
	/// A fresh world: Day 1, 0:10, at full speed.
	pub(crate) fn new() -> Self {
		let seconds_per_day: f32 = PHASES.iter().map(|p| p.duration).sum();
		WorldClock {
			time: seconds_per_day / 12.0 + seconds_per_day - 8.0,
			seconds_per_day,
			phase: 0,
			into_phase: 0.0,
			global_visibility: 1.0,
			ambient: 0xffff_ffff,
			saturation: 1.0,
			paused: false,
			speed: 1.0,
		}
	}

	/// The ambient colour (ARGB) and saturation as of the last update.
	pub(crate) fn ambient(&self) -> (u32, f32) {
		(self.ambient, self.saturation)
	}

	/// `World.clearScene()`: Day 1, 2:00, phase reset (it is recomputed on the next
	/// advance), full speed.
	pub(crate) fn clear_scene(&mut self) {
		self.time = self.seconds_per_day / 12.0 + self.seconds_per_day;
		self.phase = 0;
		self.into_phase = 0.0;
		self.global_visibility = 1.0;
		self.speed = 1.0;
	}

	/// One `World.update()` with the frame's real delta in seconds.
	pub(crate) fn advance(&mut self, real_delta: f32) {
		if self.paused {
			return;
		}
		let dt = self.speed * real_delta;
		if dt > 0.0 && self.seconds_per_day > dt {
			self.time += dt;
			self.phase = 0;
			self.into_phase = self.seconds_of_day();
			while self.into_phase > PHASES[self.phase].duration {
				self.into_phase -= PHASES[self.phase].duration;
				self.phase = (self.phase + 1) % PHASES.len();
			}
		}
		// Recomputed every update, even when time stands still.
		let current = &PHASES[self.phase];
		let next = &PHASES[(self.phase + 1) % PHASES.len()];
		let remaining = 1.0 - self.into_phase / current.duration;
		let passed = 1.0 - remaining;
		self.saturation = current.saturation * remaining + next.saturation * passed;
		self.ambient = blend_color(current.color, next.color, remaining);
		self.global_visibility = current.visibility * remaining + next.visibility * passed;
	}

	fn whole_days(&self) -> f32 {
		cvtt_i32(f64::from(self.time / self.seconds_per_day)) as f32
	}

	fn seconds_of_day(&self) -> f32 {
		self.time - self.whole_days() * self.seconds_per_day
	}

	fn seconds_per_hour(&self) -> f32 {
		self.seconds_per_day / 24.0
	}

	fn days(&self) -> i64 {
		cvtt_i32(f64::from(self.time / self.seconds_per_day)).into()
	}

	fn hours(&self) -> i64 {
		cvtt_i32(f64::from(self.seconds_of_day() / self.seconds_per_hour())).into()
	}

	fn minutes(&self) -> i64 {
		let into_hour = self.seconds_of_day() - self.hours() as f32 * self.seconds_per_hour();
		cvtt_i32(f64::from(into_hour / (self.seconds_per_hour() / 60.0))).into()
	}
}

/// A shared handle to the world clock.
pub(crate) type Clock = Rc<RefCell<WorldClock>>;

/// Build the `World` clock natives over `clock`: `(name, closure)` pairs for the
/// `World` table.
pub(crate) fn world_entries(
	vm: &mut SQVM,
	clock: &Clock,
	on_update: Rc<dyn Fn(f32, f32)>,
) -> Vec<(&'static str, SqStrongRef)> {
	// The object `World.getTime()` returns: one live view of the clock, read-only except
	// `IsPaused`.
	let ignore_f = |_: &mut Clock, _: f64| {};
	let ignore_i = |_: &mut Clock, _: i64| {};
	let ignore_b = |_: &mut Clock, _: bool| {};
	let class = ClassBuilder::<Clock>::new("WorldTime")
		.property("Time", |c: &Clock| f64::from(c.borrow().time), ignore_f)
		.property("SecondsPerDay", |c: &Clock| f64::from(c.borrow().seconds_per_day), ignore_f)
		.property("SecondsPerHour", |c: &Clock| f64::from(c.borrow().seconds_per_hour()), ignore_f)
		.property(
			"SecondsPerMinute",
			|c: &Clock| f64::from(c.borrow().seconds_per_hour() / 60.0),
			ignore_f,
		)
		.property("SecondsOfDay", |c: &Clock| f64::from(c.borrow().seconds_of_day()), ignore_f)
		.property("Days", |c: &Clock| c.borrow().days(), ignore_i)
		.property("Hours", |c: &Clock| c.borrow().hours(), ignore_i)
		.property("Minutes", |c: &Clock| c.borrow().minutes(), ignore_i)
		.property("TimeOfDay", |c: &Clock| PHASES[c.borrow().phase].time_of_day, ignore_i)
		.property("IsDaytime", |c: &Clock| PHASES[c.borrow().phase].is_daytime, ignore_b)
		.property(
			"IsPaused",
			|c: &Clock| c.borrow().paused,
			|c: &mut Clock, v: bool| c.borrow_mut().paused = v,
		)
		.build(vm);
	let time_object = class.instantiate(vm, clock.clone());
	let clock = clock.clone();

	let get_time = vm.make_closure(move || time_object.clone());
	// World.update(): advance by this frame's real delta (`Time.getDelta()`, seconds).
	let update = {
		let c = clock.clone();
		let on_update = on_update.clone();
		vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
			let time = SqTable::root(vm).get::<SqStrongRef>("Time").ok_or("Time table missing")?;
			let delta = time
				.call_method_ret::<Number, _>(vm, "getDelta", ())?
				.map_or(0.0, |n| n.0);
			c.borrow_mut().advance(delta as f32);
			// Clouds and waves move with the same update (`0x4fdc30`).
			let speed = c.borrow().speed;
			on_update(delta as f32, speed);
			Ok(())
		})
	};
	let get_speed_mult = {
		let c = clock.clone();
		vm.make_closure(move || f64::from(c.borrow().speed))
	};
	let set_speed_mult = {
		let c = clock.clone();
		vm.make_closure(move |vm: &SQVM, speed: Number| -> Result<(), String> {
			c.borrow_mut().speed = speed.0 as f32;
			let time = SqTable::root(vm).get::<SqStrongRef>("Time").ok_or("Time table missing")?;
			time.call_method(vm, "setVirtualSpeed", (f64::from(speed.0 as f32),))
		})
	};
	let clear_scene = {
		let c = clock.clone();
		vm.make_closure(move || c.borrow_mut().clear_scene())
	};

	let get_global_visibility_mult = {
		let c = clock.clone();
		vm.make_closure(move || f64::from(c.borrow().global_visibility))
	};
	let is_daytime = {
		let c = clock.clone();
		vm.make_closure(move || PHASES[c.borrow().phase].is_daytime)
	};
	vec![
		("getTime", get_time),
		("isDaytime", is_daytime),
		("getGlobalVisibilityMult", get_global_visibility_mult),
		("update", update),
		("getSpeedMult", get_speed_mult),
		("setSpeedMult", set_speed_mult),
		("clearScene", clear_scene),
	]
}

/// The world clock's share of a save.
pub(crate) struct ClockPart(pub Clock);

impl super::save::SavePart for ClockPart {
	fn save(&self, cx: &super::save::SaveCx) -> Result<(), String> {
		let c = self.0.borrow();
		let mut w = cx.writer.borrow_mut();
		w.f32(c.time);
		w.u8(c.phase as u8);
		w.f32(c.into_phase);
		w.f32(c.global_visibility);
		w.bool(c.paused);
		Ok(())
	}

	fn load(&self, cx: &super::save::LoadCx) -> Result<(), String> {
		let mut c = self.0.borrow_mut();
		let mut r = cx.reader.borrow_mut();
		c.time = r.f32()?;
		c.phase = usize::from(r.u8()?).min(PHASES.len() - 1);
		c.into_phase = r.f32()?;
		c.global_visibility = r.f32()?;
		c.paused = r.bool()?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// (Time, Days, Hours, Minutes, TimeOfDay) as probed in the original.
	fn read(c: &WorldClock) -> (f32, i64, i64, i64, i64) {
		(c.time, c.days(), c.hours(), c.minutes(), PHASES[c.phase].time_of_day)
	}

	#[test]
	fn matches_the_original_clock() {
		let mut c = WorldClock::new();
		assert_eq!(c.seconds_per_day, 105.0);
		assert_eq!(c.seconds_per_hour(), 4.375);
		assert_eq!(read(&c), (105.75, 1, 0, 10, 0));
		// Probed: one real second at 2x each.
		c.speed = 2.0;
		c.advance(1.0);
		assert_eq!(read(&c), (107.75, 1, 0, 37, 0));
		c.advance(1.0);
		assert_eq!(read(&c), (109.75, 1, 1, 5, 1));
		// Paused (speed 0): no change.
		c.speed = 0.0;
		c.advance(1.0);
		assert_eq!(read(&c), (109.75, 1, 1, 5, 1));
	}

	#[test]
	fn walks_the_day_phases() {
		let mut c = WorldClock::new();
		// From 0.75 s into the day to 76 s: Night.
		c.advance(75.25);
		assert_eq!(PHASES[c.phase].time_of_day, 6);
		assert!(!PHASES[c.phase].is_daytime);
		// Early morning: 100 s into the day.
		c.advance(24.0);
		assert_eq!(PHASES[c.phase].time_of_day, 7);
		assert!(PHASES[c.phase].is_daytime);
		// A frame as long as the day itself is ignored, like in the original.
		let before = c.time;
		c.advance(105.0);
		assert_eq!(c.time, before);
	}

	#[test]
	fn visibility_blends_into_the_night() {
		let mut c = WorldClock::new();
		// Exactly 65 s into the day is still the end of Evening (a strict comparison).
		c.advance(64.25);
		assert_eq!(PHASES[c.phase].time_of_day, 4);
		// Half a second into Dusk: Dusk's 0.75 throughout.
		c.advance(0.5);
		assert_eq!(PHASES[c.phase].time_of_day, 5);
		assert_eq!(c.global_visibility, 0.75);
		// Halfway through the second Dusk phase: halfway from 0.75 to Night's 0.5.
		c.advance(7.0);
		assert_eq!(c.global_visibility, 0.625);
	}

	#[test]
	fn lighting_blends_between_phases() {
		let mut c = WorldClock::new();
		assert_eq!(c.ambient(), (0xffff_ffff, 1.0), "neutral until the clock runs");
		c.paused = true;
		c.advance(1.0);
		assert_eq!(c.ambient(), (0xffff_ffff, 1.0), "paused: nothing recomputed");
		c.paused = false;
		// 1.75 s into Dawn (#ffa462), 7/12 of the way to Morning (#ffb762).
		c.advance(1.0);
		assert_eq!(c.ambient().0, blend_color(0xffffa462, 0xffffb762, 1.0 - 1.75 / 3.0));
		assert_eq!(c.ambient().0 & 0xffff_00ff, 0xffff_0062);
		// Deep night, 3 s into the first Night phase: Night's own colour.
		c.advance(76.0 - 1.75 - 3.0 + 3.0);
		assert_eq!(c.ambient(), (0xff7083a8, 0.8));
	}

	#[test]
	fn colours_blend_per_channel_and_truncate() {
		assert_eq!(blend_color(0xff000000, 0xff0000ff, 0.5), 0xff00007f);
		assert_eq!(blend_color(0xffffffff, 0xffffffff, 0.3), 0xffffffff);
		assert_eq!(blend_color(0xff102030, 0xff102030, 0.0), 0xff102030);
	}

	#[test]
	fn clear_scene_restarts_at_two_o_clock() {
		let mut c = WorldClock::new();
		c.speed = 0.0;
		c.clear_scene();
		assert_eq!(read(&c), (113.75, 1, 2, 0, 0));
		assert_eq!(c.speed, 1.0);
	}
}
