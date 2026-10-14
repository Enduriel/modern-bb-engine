//! The battle camera's movement callbacks, as the original's camera class
//! (disassembled 2026-10-14):
//! - `registerCallbacks(start, end)` (`0x4a4ab0`) adds a function to each of two lists,
//!   `unregisterCallbacks(start, end)` (`0x4a4b70`) takes them out again;
//! - `update()` (`0x4a3f10`), which the tactical state calls every frame, calls each
//!   `start` function once the camera has begun to move since the last update, and each
//!   `end` function once it has stopped (a null one is skipped).
//!
//! The tactical state's `start` hides the tooltip. With the instant camera (headless)
//! the camera never moves, so neither list is ever called.

use squirrel::{ClassBuilder, Retained, SQVM, SqStrongRef};

use super::Grid;
use super::camera::CameraHandle;
use crate::native::camera_motion::Camera;

/// The two lists, and whether the camera was moving at the last `update`.
#[derive(Default)]
pub(crate) struct MovementCallbacks {
	start: Vec<SqStrongRef>,
	end: Vec<SqStrongRef>,
	was_moving: bool,
}

/// Add the three natives. They run scripts, so they reach the camera through `grid`
/// rather than the instance's payload.
pub(super) fn add(b: ClassBuilder<CameraHandle>, grid: &Grid) -> ClassBuilder<CameraHandle> {
	let register = {
		let grid = grid.clone();
		move |start: Retained, end: Retained| {
			let callbacks = &mut grid.borrow_mut().camera.callbacks;
			callbacks.start.push(start.0);
			callbacks.end.push(end.0);
		}
	};
	let unregister = {
		let grid = grid.clone();
		move |start: Retained, end: Retained| {
			let callbacks = &mut grid.borrow_mut().camera.callbacks;
			let (start, end) = (start.0.identity(), end.0.identity());
			callbacks.start.retain(|f| f.identity() != start);
			callbacks.end.retain(|f| f.identity() != end);
		}
	};
	let update = {
		let grid = grid.clone();
		move |vm: &SQVM| {
			let due = {
				let mut g = grid.borrow_mut();
				let moving = g.camera.glide.as_ref().is_some_and(Camera::is_moving);
				let callbacks = &mut g.camera.callbacks;
				match (std::mem::replace(&mut callbacks.was_moving, moving), moving) {
					(false, true) => callbacks.start.clone(),
					(true, false) => callbacks.end.clone(),
					_ => return,
				}
			};
			for f in due {
				// The original reports a throw (the VM's error handler prints it) and
				// goes on.
				let _ = f.call(vm, ());
			}
		}
	};
	b.native_method("registerCallbacks", register)
		.native_method("unregisterCallbacks", unregister)
		.native_method("update", update)
}
