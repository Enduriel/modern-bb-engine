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
//!
//! Event lifetime, as probed and disassembled: the engine never drops a
//! pending event by itself (not on `World.clearScene`, `Tactical.clearScene`, or a new
//! campaign); only `Time.clearEvents()` does, for every unit at once. The states'
//! scripts call it on leaving a campaign, before a load and when a battle ends. Virtual
//! events freeze while the virtual speed is 0 and keep their absolute deadline when the
//! clock is reset or set.
//!
//! The engine's own timers share the list (disassembled, `0x5445f0`'s callers; probed):
//! the ambience's next sound per layer and a silent music's retry, both
//! Real. So `clearEvents` cancels them too, and `hasEventScheduled(TimeUnit.Real)` and
//! `(4)` see them (see `audio`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use squirrel::{Number, Retained, SQVM, SqStrongRef, SqTable, This};

// The script-facing `TimeUnit` values.
const UNIT_REAL: i64 = 0;
const UNIT_VIRTUAL: i64 = 1;
const UNIT_ROUNDS: i64 = 2;

// The original stores an event under its own internal unit, which `hasEventScheduled`
// compares the script's value against (disassembled, 0x51d2d0/0x51d310): Real 0,
// Virtual 1, frames 2 (engine-internal events only) and Rounds 3. Value 4 asks for
// "any event".
const KIND_REAL: i64 = 0;
const KIND_VIRTUAL: i64 = 1;
const KIND_ROUNDS: i64 = 3;
const HAS_ANY: i64 = 4;

/// The clock behind `Time.getExactTime()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExactClock {
	/// The frame's real time, like `getRealTimeF`: deterministic, for the harness and
	/// tests (draw-exact battles depend on it). Work budgeted with it never runs out
	/// within a frame, so the AI's evaluations never yield for time.
	#[default]
	Frame,
	/// A live monotonic clock (seconds since the service started), as the original
	/// has: the AI's background work then yields after its per-frame budget (see
	/// `RootState::run_background`).
	Live,
}

/// A scheduled callback: fires when its clock reaches `fire_threshold`.
struct Event {
	/// The internal unit (`KIND_*`), not the script's `TimeUnit` value.
	kind: i64,
	fire_threshold: f64,
	action: Action,
}

/// What an event does when it fires.
enum Action {
	/// A script's `callback(userdata)`, with `this` the receiver `scheduleEvent` was
	/// called on (`Time`, normally), as Sqrat keeps it (probed).
	Script { callback: SqStrongRef, this: SqStrongRef, userdata: SqStrongRef },
	/// The engine's own (the music's and the ambience's timers): the original keeps them
	/// on the same list, so `clearEvents` drops them and `hasEventScheduled` sees them.
	Native(Box<dyn FnOnce()>),
}

#[derive(Default)]
struct TimeState {
	real_time: f64,     // ms since boot, accumulated from `tick`
	virtual_time: f64,  // ms of in-game time
	virtual_speed: f64, // multiplier applied to real delta (1.0 = real-time)
	round: i64,
	frame: i64,
	last_delta: f64, // real ms of the last frame
	/// Pending events in scheduling order, the order a pass fires them in.
	events: Vec<Event>,
	/// While a pass fires events (`firing`), new ones wait here and join `events`
	/// after it, so none fires in the pass that scheduled it.
	deferred: Vec<Event>,
	firing: bool,
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
#[derive(Clone)]
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
	/// milliseconds (the host supplies this from its own clock), then fire every due
	/// event of all three units. Call once per frame.
	///
	/// As the original's pass (`0x5443e0`): one walk over the pending events in
	/// scheduling order, firing each due one as it is reached (Rounds events too:
	/// `setRound` only sets the round, they fire here). Events scheduled by a callback
	/// wait for the next pass. A callback that calls `Time.clearEvents()` drops the
	/// rest of the pass as well.
	pub fn tick(&self, vm: &SQVM, real_delta_ms: f64) {
		{
			let mut st = self.state.borrow_mut();
			st.real_time += real_delta_ms;
			st.frame += 1;
			st.last_delta = real_delta_ms;
			st.virtual_time += st.virtual_speed * real_delta_ms;
			st.firing = true;
		}
		let mut i = 0;
		loop {
			let due = {
				let mut st = self.state.borrow_mut();
				let (real_now, virtual_now, round_now) = (st.real_time, st.virtual_time, st.round as f64);
				let is_due = |e: &Event| match e.kind {
					KIND_REAL => real_now >= e.fire_threshold,
					KIND_VIRTUAL => virtual_now >= e.fire_threshold,
					_ => round_now >= e.fire_threshold, // KIND_ROUNDS
				};
				while i < st.events.len() && !is_due(&st.events[i]) {
					i += 1;
				}
				if i >= st.events.len() {
					break;
				}
				st.events.remove(i)
			};
			match due.action {
				// A throwing callback must not abort the frame.
				Action::Script { callback, this, userdata } => drop(callback.call_with_this(vm, &this, (userdata,))),
				Action::Native(run) => run(),
			}
		}
		let mut st = self.state.borrow_mut();
		st.firing = false;
		let deferred = std::mem::take(&mut st.deferred);
		st.events.extend(deferred);
	}

	/// Schedule the engine's own `run` on the Real clock, `delay_ms` after the clock's
	/// whole milliseconds, among the scripts' events (see [`Action::Native`]).
	pub(crate) fn schedule_real(&self, delay_ms: u32, run: impl FnOnce() + 'static) {
		let mut st = self.state.borrow_mut();
		let fire_threshold = st.real_time.trunc() + f64::from(delay_ms);
		st.push(Event { kind: KIND_REAL, fire_threshold, action: Action::Native(Box::new(run)) });
	}

	/// The Real clock in whole milliseconds, as the original's engine reads it
	/// (`[Time + 4]`): its sounds' fades and timers run on it.
	pub(crate) fn real_ms(&self) -> u32 {
		self.state.borrow().real_time as u32
	}

	/// Choose the clock behind `getExactTime` (default [`ExactClock::Frame`]).
	pub fn set_exact_clock(&self, clock: ExactClock) {
		self.state.borrow_mut().live_origin = match clock {
			ExactClock::Frame => None,
			ExactClock::Live => Some(Instant::now()),
		};
	}

	/// Number of scheduled events not yet fired (host/test inspection).
	pub fn pending_events(&self) -> usize {
		let st = self.state.borrow();
		st.events.len() + st.deferred.len()
	}

	/// Whether the scripts' own events answer `Time.hasEventScheduled(unit)`, leaving out
	/// the engine's (the music's and the ambience's timers; host/test inspection).
	pub fn script_event_scheduled(&self, unit: i64) -> bool {
		let st = self.state.borrow();
		st.events.iter().any(|e| matches!(e.action, Action::Script { .. }) && (unit == HAS_ANY || e.kind == unit))
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

	/// Set the virtual time to `seconds` (a loaded save's; tests, a long campaign's).
	pub fn set_virtual_seconds(&self, seconds: f64) {
		self.state.borrow_mut().virtual_time = seconds * 1000.0;
	}

	/// Virtual time back to 0 at speed 1, as `World.clearScene()` sets it (`0x4eff3a`).
	pub(crate) fn reset_virtual(&self) {
		let mut st = self.state.borrow_mut();
		st.virtual_time = 0.0;
		st.virtual_speed = 1.0;
	}

	/// Drop all pending scheduled events (releasing their callback/userdata
	/// references). Call before closing the VM if the service might outlive it. The
	/// same as the script-side `Time.clearEvents()`.
	pub fn clear(&self) {
		// Taken out first: dropping an event's references may run script finalizers.
		let dropped = {
			let mut st = self.state.borrow_mut();
			(std::mem::take(&mut st.events), std::mem::take(&mut st.deferred))
		};
		drop(dropped);
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
			// Drops every pending event, of all units, without calling it (0x5446b0;
			// the original's only caller is this native, so nothing else in the engine
			// drops events: the states' scripts call it on leaving a campaign or a
			// battle and before a load).
			("clearEvents", {
				let service = self.clone();
				vm.make_closure(move || service.clear())
			}),
			// Compares against the internal unit (see `KIND_*`), so `Rounds` (2) asks
			// for frame events and is false for scheduled Rounds events, 3 finds those,
			// and 4 is "any event". Events scheduled during the running pass don't
			// count yet (disassembled, 0x51d2d0). The engine's timers count as well:
			// with the world's ambience running, Real and 4 are always true (probed).
			("hasEventScheduled", {
				let state = self.state.clone();
				vm.make_closure(move |unit: i64| {
					let st = state.borrow();
					if unit == HAS_ANY {
						!st.events.is_empty()
					} else {
						st.events.iter().any(|e| e.kind == unit)
					}
				})
			}),
			// Only sets the round: due Rounds events fire in the next frame's pass
			// (0x51d290).
			("setRound", {
				let state = self.state.clone();
				vm.make_closure(move |round: i64| state.borrow_mut().round = round)
			}),
			("scheduleEvent", {
				let state = self.state.clone();
				vm.make_closure(
					move |vm: &SQVM, this: This, unit: i64, delay: Number, callback: SqStrongRef, userdata: Retained| {
						let mut st = state.borrow_mut();
						// Any other unit schedules nothing (0x51d310).
						let kind = match unit {
							UNIT_REAL => KIND_REAL,
							UNIT_VIRTUAL => KIND_VIRTUAL,
							UNIT_ROUNDS => KIND_ROUNDS,
							_ => return,
						};
						// Whole milliseconds (rounds), as the original: the clock's integer
						// value (what getVirtualTime/getRealTime read) plus the delay
						// truncated (disassembled). So an AI that waits `getVirtualTime() +
						// d` and a hit due `d` later fall due together, and the event
						// fires first (events fire at the start of the frame).
						let base = match kind {
							KIND_VIRTUAL => st.virtual_time.trunc(),
							KIND_ROUNDS => st.round as f64,
							_ => st.real_time.trunc(),
						};
						let action = Action::Script { callback, this: this.to_strong(vm), userdata: userdata.0 };
						st.push(Event { kind, fire_threshold: base + delay.0.trunc(), action });
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

impl TimeState {
	/// A new event: on the list, or on the side list while a pass runs.
	fn push(&mut self, event: Event) {
		if self.firing {
			self.deferred.push(event);
		} else {
			self.events.push(event);
		}
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
