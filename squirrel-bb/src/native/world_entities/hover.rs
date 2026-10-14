//! The tile under the mouse on the world map. The original's world keeps a pointer to
//! it (`World+0xd4`), set by the world's own mouse listener (`0x4f4550`) on every
//! pointer move whose tile is on the map; a move off the map keeps the last one. The
//! one-argument `World.spawnEntity(script)` and `World.spawnLocation(script)` spawn
//! there (`0x4f1910`).

use std::cell::Cell;
use std::rc::Rc;

use squirrel::SQVM;

use crate::input_events::MouseInput;
use crate::native::screen::Screen;
use crate::native::world_camera::WorldCamera;
use crate::native::world_layout::world_to_tile;

/// The hovered world tile, shared by the mouse listener and the spawn natives.
#[derive(Clone, Default)]
pub(crate) struct Hover(Rc<Cell<Option<(i64, i64)>>>);

impl Hover {
	/// The hovered tile's axial coords, once the pointer has moved over the map.
	pub(crate) fn tile(&self) -> Option<(i64, i64)> {
		self.0.get()
	}

	/// The world's mouse listener: on a pointer move, keep the tile under the pointer
	/// (through the world camera) if it is on the `(width, height)` map.
	pub(crate) fn on_mouse(&self, vm: &SQVM, camera: &WorldCamera, (width, height): (i64, i64), input: &MouseInput) {
		if input.state != MouseInput::moved(0, 0).state {
			return;
		}
		let view = *camera.borrow();
		let (x, y) = Screen::of(vm).to_world(view.pos, view.zoom, input.x as i32, input.y as i32);
		let (q, r) = world_to_tile(f64::from(x), f64::from(y), width, height);
		if (0..width).contains(&q) && (0..height).contains(&(r + q / 2)) {
			self.0.set(Some((q, r)));
		}
	}
}
