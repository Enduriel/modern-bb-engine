//! The camera class's movement callbacks, shared by the world's camera and the battle's
//! (one class in the original, disassembled 2026-10-14):
//! - `registerCallbacks(start, end)` (`0x4a4ab0`) adds a function to each of two lists,
//!   `unregisterCallbacks(start, end)` (`0x4a4b70`) takes them out again;
//! - `update()` (`0x4a3f10`) calls each `start` function once the camera has begun to
//!   move since the last update, and each `end` function once it has stopped (a null one
//!   is skipped), with `this` the camera they were registered on (probed: Sqrat keeps a
//!   function argument's receiver as its environment).
//!
//! The lists hold script functions: their owner clears them before the VM closes.

use squirrel::{SQVM, SqStrongRef};

/// The two lists, and whether the camera was moving at the last `update`.
#[derive(Default)]
pub(crate) struct MovementCallbacks {
	/// Each function, with the receiver `registerCallbacks` was called on.
	start: Vec<(SqStrongRef, SqStrongRef)>,
	end: Vec<(SqStrongRef, SqStrongRef)>,
	was_moving: bool,
}

impl MovementCallbacks {
	/// `registerCallbacks(start, end)`, called on `this`.
	pub(crate) fn register(&mut self, this: SqStrongRef, start: SqStrongRef, end: SqStrongRef) {
		self.start.push((start, this.clone()));
		self.end.push((end, this));
	}

	/// `unregisterCallbacks(start, end)`: each list loses its function.
	pub(crate) fn unregister(&mut self, start: &SqStrongRef, end: &SqStrongRef) {
		let (start, end) = (start.identity(), end.identity());
		self.start.retain(|(f, _)| f.identity() != start);
		self.end.retain(|(f, _)| f.identity() != end);
	}

	/// What an `update` with the camera `moving` calls: the start list as it begins to
	/// move, the end list as it stops, else nothing.
	pub(crate) fn due(&mut self, moving: bool) -> Vec<(SqStrongRef, SqStrongRef)> {
		match (std::mem::replace(&mut self.was_moving, moving), moving) {
			(false, true) => self.start.clone(),
			(true, false) => self.end.clone(),
			_ => Vec::new(),
		}
	}

	/// Release the functions. Call before the VM is closed.
	pub(crate) fn clear(&mut self) {
		self.start.clear();
		self.end.clear();
	}
}

/// Call the functions an `update` found due, each with its camera as `this`. The
/// original reports a throw (the VM's error handler prints it) and goes on (probed).
pub(crate) fn fire(vm: &SQVM, due: Vec<(SqStrongRef, SqStrongRef)>) {
	for (f, this) in due {
		let _ = f.call_with_this(vm, &this, ());
	}
}
