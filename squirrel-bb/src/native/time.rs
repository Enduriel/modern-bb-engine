//! The `Time` service: three clocks (Real / Virtual / Rounds), the `TimeUnit`
//! table, and `scheduleEvent`.
//!
//! Unlike the stateless natives, `Time` is **stateful and host-driven**: the host
//! owns a [`TimeService`], installs it, and advances it each frame via
//! [`tick`](TimeService::tick) (the engine supplies the real-time delta — this
//! crate has no clock of its own). The Rounds clock is driven by scripts calling
//! `Time.setRound`. Scheduled events fire when their clock passes the deadline.
//!
//! Units, as probed in the original:
//! - `getRealTime`/`getVirtualTime` are integer **milliseconds**; `getRealTimeF`,
//!   `getVirtualTimeF`, `getExactTime` and `getDelta` are **seconds**.
//! - `scheduleEvent` Real/Virtual delays are milliseconds, relative to the current
//!   clock value in whole milliseconds (the delay truncated too; disassembled).
//! - The virtual clock runs at `getVirtualSpeed()` times real time; `World.setSpeedMult`
//!   sets that speed (0 while the game is paused).
//! - `getVirtualDelta` is assumed to be the frame's real delta times the virtual speed
//!   (seconds); not yet checked against the original.
//! - A scheduled callback is invoked as `callback(userdata)` with `this` = the
//!   root table (a `bindenv`'d callback uses its bound environment instead).
//! - `getExactTime` is a live clock in the original: it moves within a frame
//!   (probed), and the AI's background task budgets its work with it (~5 ms a frame).
//!   Which clock it reads here is the host's choice ([`ExactClock`]).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use squirrel::{Number, Retained, SQVM, SqStrongRef, SqTable};

const UNIT_REAL: i64 = 0;
const UNIT_VIRTUAL: i64 = 1;
const UNIT_ROUNDS: i64 = 2;

/// The clock behind `Time.getExactTime()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExactClock {
	/// The frame's real time, like `getRealTimeF`: deterministic, for the harness and
	/// tests (draw-exact battles depend on it). Work budgeted with it never runs out
	/// within a frame, so the AI evaluates a whole turn at once.
	#[default]
	Frame,
	/// A live monotonic clock (seconds since the service started), as the original
	/// has: the AI's background work then yields after its per-frame budget.
	Live,
}

/// A scheduled callback: fires when its clock reaches `fire_threshold`.
struct Event {
	unit: i64,
	fire_threshold: f64,
	callback: SqStrongRef,
	userdata: SqStrongRef,
}

#[derive(Default)]
struct TimeState {
	real_time: f64,     // ms since boot, accumulated from `tick`
	virtual_time: f64,  // ms of in-game time
	virtual_speed: f64, // multiplier applied to real delta (1.0 = real-time)
	round: i64,
	frame: i64,
	last_delta: f64, // real ms of the last frame
	events: Vec<Event>,
	/// Set for [`ExactClock::Live`]: `getExactTime` counts from here.
	live_origin: Option<Instant>,
}

/// The host-owned `Time` service. Construct one, [`install`](Self::install) it
/// before running scripts, and drive it with [`tick`](Self::tick) each frame.
///
/// # Drop order
/// Pending events hold [`SqStrongRef`]s, which release against the VM on drop, so
/// a `TimeService` must be dropped **before** its [`SQVM`] is closed (releasing
/// after `sq_close` is a use-after-free). The host naturally satisfies this by
/// owning the VM at the top and dropping the service first; [`clear`](Self::clear)
/// can drop all pending events explicitly while the VM is still live.
pub struct TimeService {
	state: Rc<RefCell<TimeState>>,
}

impl Default for TimeService {
	fn default() -> Self {
		Self::new()
	}
}

impl TimeService {
	/// A fresh service: all clocks at zero, virtual speed 1.0.
	pub fn new() -> Self {
		TimeService {
			state: Rc::new(RefCell::new(TimeState {
				virtual_speed: 1.0,
				..TimeState::default()
			})),
		}
	}

	/// Advance the Real and Virtual clocks by a frame of `real_delta_ms` real
	/// milliseconds (the host supplies this from its own clock), then fire any due
	/// Real/Virtual events. Call once per frame.
	pub fn tick(&self, vm: &SQVM, real_delta_ms: f64) {
		let due = {
			let mut st = self.state.borrow_mut();
			st.real_time += real_delta_ms;
			st.frame += 1;
			st.last_delta = real_delta_ms;
			st.virtual_time += st.virtual_speed * real_delta_ms;
			let (real_now, virtual_now) = (st.real_time, st.virtual_time);
			drain_due(&mut st.events, |e| match e.unit {
				UNIT_REAL => real_now >= e.fire_threshold,
				UNIT_VIRTUAL => virtual_now >= e.fire_threshold,
				_ => false,
			})
		};
		fire_all(vm, due);
	}

	/// Choose the clock behind `getExactTime` (default [`ExactClock::Frame`]).
	pub fn set_exact_clock(&self, clock: ExactClock) {
		self.state.borrow_mut().live_origin = match clock {
			ExactClock::Frame => None,
			ExactClock::Live => Some(Instant::now()),
		};
	}

	/// Real time since boot in seconds (`Time.getRealTimeF()`).
	pub fn real_seconds(&self) -> f64 {
		self.state.borrow().real_time / 1000.0
	}

	/// Virtual (game) time in seconds.
	pub fn virtual_seconds(&self) -> f64 {
		self.state.borrow().virtual_time / 1000.0
	}

	/// The clocks' share of a save: virtual time and the round. (Real time keeps
	/// running; scheduled events hold script callbacks and are cleared on load by the
	/// world state.)
	pub(crate) fn save_part(&self) -> Rc<dyn super::save::SavePart> {
		Rc::new(TimePart(self.state.clone()))
	}

	/// Drop all pending scheduled events (releasing their callback/userdata
	/// references). Call before closing the VM if the service might outlive it. The
	/// same as the script-side `Time.clearEvents()`.
	pub fn clear(&self) {
		self.state.borrow_mut().events.clear();
	}

	/// Install the `Time` table and the `TimeUnit` enum into the root.
	pub fn install(&self, vm: &mut SQVM) {
		// Each getter reads a value out of the shared clock state. A local macro
		// expands to a concrete closure (the return-conversion trait is sealed, so
		// a generic helper can't name the bound).
		macro_rules! getter {
			(|$st:ident| $body:expr) => {{
				let state = self.state.clone();
				vm.make_closure(move || {
					let $st = state.borrow();
					$body
				})
			}};
		}

		let entries: Vec<(&str, SqStrongRef)> = vec![
			("getRealTime", getter!(|st| st.real_time as i64)),
			("getRealTimeF", getter!(|st| st.real_time / 1000.0)),
			(
				"getExactTime",
				getter!(|st| st.live_origin.map_or(st.real_time / 1000.0, |o| o.elapsed().as_secs_f64())),
			),
			("getVirtualTime", getter!(|st| st.virtual_time as i64)),
			("getVirtualTimeF", getter!(|st| st.virtual_time / 1000.0)),
			(
				"getVirtualDelta",
				getter!(|st| st.last_delta * st.virtual_speed / 1000.0),
			),
			("getDelta", getter!(|st| st.last_delta / 1000.0)),
			("getVirtualSpeed", getter!(|st| st.virtual_speed)),
			("getFrame", getter!(|st| st.frame)),
			("getRound", getter!(|st| st.round)),
			(
				"getFPS",
				getter!(|st| if st.last_delta > 0.0 {
					1000.0 / st.last_delta
				} else {
					0.0
				}),
			),
			("setVirtualSpeed", {
				let state = self.state.clone();
				vm.make_closure(move |speed: Number| state.borrow_mut().virtual_speed = speed.0)
			}),
			// setVirtualTime(seconds), as getVirtualTimeF reads them: the world parks its
			// clock in seconds while a battle runs on one from 0 and puts it back after
			// (disassembled, 0x51d260: the f32 argument becomes the seconds clock, and
			// trunc(× 1000) the millisecond one).
			("setVirtualTime", {
				let state = self.state.clone();
				vm.make_closure(move |seconds: Number| state.borrow_mut().virtual_time = seconds.0 as f32 as f64 * 1000.0)
			}),
			("clearEvents", {
				let state = self.state.clone();
				vm.make_closure(move || state.borrow_mut().events.clear())
			}),
			("hasEventScheduled", {
				let state = self.state.clone();
				vm.make_closure(move |unit: i64| {
					state.borrow().events.iter().any(|e| e.unit == unit)
				})
			}),
			("setRound", {
				let state = self.state.clone();
				vm.make_closure(move |vm: &SQVM, round: i64| {
					// Setting the round fires any due Rounds events.
					let due = {
						let mut st = state.borrow_mut();
						st.round = round;
						let now = round as f64;
						drain_due(&mut st.events, |e| {
							e.unit == UNIT_ROUNDS && now >= e.fire_threshold
						})
					};
					fire_all(vm, due);
				})
			}),
			("scheduleEvent", {
				let state = self.state.clone();
				vm.make_closure(
					move |unit: i64, delay: Number, callback: SqStrongRef, userdata: Retained| {
						let mut st = state.borrow_mut();
						// Whole milliseconds (rounds), as the original: the clock's integer
						// value (what getVirtualTime/getRealTime read) plus the delay
						// truncated (disassembled). So an AI that waits `getVirtualTime() +
						// d` and a hit due `d` later fall due together, and the event
						// fires first (events fire at the start of the frame).
						let base = match unit {
							UNIT_VIRTUAL => st.virtual_time.trunc(),
							UNIT_ROUNDS => st.round as f64,
							_ => st.real_time.trunc(), // UNIT_REAL and anything else
						};
						st.events.push(Event {
							unit,
							fire_threshold: base + delay.0.trunc(),
							callback,
							userdata: userdata.0,
						});
					},
				)
			}),
		];

		let mut time = SqTable::new(vm);
		for (name, func) in entries {
			time.set(name, func);
		}
		SqTable::root(vm).set("Time", time);

		let mut time_unit = SqTable::new(vm);
		time_unit.set("Real", UNIT_REAL);
		time_unit.set("Virtual", UNIT_VIRTUAL);
		time_unit.set("Rounds", UNIT_ROUNDS);
		SqTable::root(vm).set("TimeUnit", time_unit);
	}
}

/// Remove and return the `(callback, userdata)` of every event matching `due`.
fn drain_due(
	events: &mut Vec<Event>,
	due: impl Fn(&Event) -> bool,
) -> Vec<(SqStrongRef, SqStrongRef)> {
	let mut fired = Vec::new();
	let mut i = 0;
	while i < events.len() {
		if due(&events[i]) {
			let e = events.remove(i);
			fired.push((e.callback, e.userdata));
		} else {
			i += 1;
		}
	}
	fired
}

/// Invoke each due callback as `callback(userdata)`. Errors are swallowed: a
/// scheduled callback that throws must not abort the frame.
fn fire_all(vm: &SQVM, due: Vec<(SqStrongRef, SqStrongRef)>) {
	for (callback, userdata) in due {
		let _ = callback.call(vm, (userdata,));
	}
}

struct TimePart(Rc<RefCell<TimeState>>);

impl super::save::SavePart for TimePart {
	fn save(&self, cx: &super::save::SaveCx) -> Result<(), String> {
		let st = self.0.borrow();
		let mut w = cx.writer.borrow_mut();
		w.f64(st.virtual_time);
		w.i64(st.round);
		Ok(())
	}

	fn load(&self, cx: &super::save::LoadCx) -> Result<(), String> {
		let mut st = self.0.borrow_mut();
		let mut r = cx.reader.borrow_mut();
		st.virtual_time = r.f64()?;
		st.round = r.i64()?;
		Ok(())
	}
}
