//! The battle camera's movement callbacks (see `native::camera_callbacks`): the lists live
//! in the grid's camera, which the grid clears before the VM closes. The tactical state's
//! `start` hides the tooltip. With the instant camera (headless) the camera never moves,
//! so neither list is ever called.

use squirrel::{ClassBuilder, Retained, SQVM};

use super::Grid;
use super::camera::CameraHandle;
use crate::native::camera_callbacks;
use crate::native::camera_motion::Camera;

/// Add the three natives. They run scripts, so they reach the camera through `grid`
/// rather than the instance's payload.
pub(super) fn add(b: ClassBuilder<CameraHandle>, grid: &Grid) -> ClassBuilder<CameraHandle> {
	let register = {
		let grid = grid.clone();
		move |start: Retained, end: Retained| grid.borrow_mut().camera.callbacks.register(start.0, end.0)
	};
	let unregister = {
		let grid = grid.clone();
		move |start: Retained, end: Retained| grid.borrow_mut().camera.callbacks.unregister(&start.0, &end.0)
	};
	let update = {
		let grid = grid.clone();
		move |vm: &SQVM| {
			let due = {
				let mut g = grid.borrow_mut();
				let moving = g.camera.glide.as_ref().is_some_and(Camera::is_moving);
				g.camera.callbacks.due(moving)
			};
			camera_callbacks::fire(vm, due);
		}
	};
	b.native_method("registerCallbacks", register)
		.native_method("unregisterCallbacks", unregister)
		.native_method("update", update)
}
