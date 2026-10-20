//! A script profiler: where a VM's time goes, by function, with call counts and the call
//! tree.
//!
//! [`SQVM::profiled`] runs a closure with the profiler attached and adds what ran in it
//! to a [`ScriptProfiler`]. Script functions are seen through the VM's native debug hook
//! (a call and a return event per Squirrel call), native functions registered through
//! this crate through their trampoline. Each event reads the clock, and the time the
//! profiler spends on an event is left out of every function's time, so what's left of
//! its cost is the VM's own dispatch to the hook (measure a run with and without
//! profiling to see it).
//!
//! Not attached, it costs nothing in the VM (no hook is set) and one relaxed atomic load
//! per native call.
//!
//! Caveats: profiling replaces a script's debug hook (`setdebughook`) for its duration
//! and leaves none set after, so don't profile while a script relies on one (Modern Hooks
//! sets one only while it boots). A generator's `yield` has no event: the generator's
//! frame stays open until its caller returns (counted in [`Profile::unmatched`]). Natives
//! of the standard library (`format`, `split`, …) don't go through the trampoline, so
//! their time is their caller's.

mod hash;
mod hook;
mod recorder;
#[cfg(test)]
mod tests;

pub(crate) use hook::{native_enter, native_exit};

use crate::SQVM;
use recorder::Recorder;

/// What a profiled function is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionKind {
	/// A Squirrel function.
	Script,
	/// A native function registered through this crate; its name is the Rust closure's
	/// type (which names the function that registered it).
	Native,
}

/// One function's totals over everything profiled.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfiledFunction {
	pub kind: FunctionKind,
	/// The function's name (`None` for an anonymous script function).
	pub name: Option<String>,
	/// The source it was compiled from (`None` for a native, or a script without one).
	pub source: Option<String>,
	/// The line of its first instruction (0 for a native).
	pub line: i64,
	pub calls: u64,
	/// Time in the function itself, not in the functions it called (nanoseconds).
	pub self_ns: u64,
	/// Time from call to return, callees included, counted once through recursion.
	pub total_ns: u64,
}

/// A call path: a function as called from its parent's path.
#[derive(Clone, Debug, PartialEq)]
pub struct CallNode {
	/// Index into [`Profile::functions`].
	pub function: usize,
	/// The caller's node (`None` at the top: called from outside any profiled function).
	pub parent: Option<usize>,
	pub calls: u64,
	pub self_ns: u64,
	pub total_ns: u64,
}

/// What a [`ScriptProfiler`] has seen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
	pub functions: Vec<ProfiledFunction>,
	/// Every call path seen; a parent comes before its children.
	pub nodes: Vec<CallNode>,
	/// Time inside [`SQVM::profiled`], the profiler's own cost left out (nanoseconds).
	pub wall_ns: u64,
	/// Time outside any function (the profiled closure's own code), nanoseconds.
	pub outside_ns: u64,
	/// The profiler's own time on events, left out of every figure (nanoseconds).
	pub overhead_ns: u64,
	/// Call and return events seen.
	pub events: u64,
	/// Returns that didn't close the innermost open call (a generator's yield left a
	/// frame open, or an event came without its pair).
	pub unmatched: u64,
	/// How many [`SQVM::profiled`] runs this covers.
	pub runs: u64,
}

/// Collects profiles over any number of [`SQVM::profiled`] runs.
#[derive(Default)]
pub struct ScriptProfiler {
	recorder: Recorder,
}

impl ScriptProfiler {
	pub fn new() -> Self {
		Self::default()
	}

	/// Everything profiled so far.
	pub fn profile(&self) -> Profile {
		self.recorder.profile()
	}

	/// How many [`SQVM::profiled`] runs it has seen (cheaper than [`profile`](Self::profile)).
	pub fn profile_runs(&self) -> u64 {
		self.recorder.runs()
	}

	/// Forget everything profiled so far.
	pub fn clear(&mut self) {
		self.recorder = Recorder::default();
	}
}

impl SQVM {
	/// Run `f` with `profiler` attached to this VM: every script call and native call
	/// made in it (on this VM's main thread) is timed and added to `profiler`. Calls
	/// still open when `f` returns are closed then. Profiling isn't nested: a
	/// `profiled` inside another's `f` just runs its `f`.
	pub fn profiled<R>(&self, profiler: &mut ScriptProfiler, f: impl FnOnce() -> R) -> R {
		let Some(_attached) = hook::Attached::new(self, &mut profiler.recorder) else { return f() };
		f()
	}
}
