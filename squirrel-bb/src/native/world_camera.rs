//! `World.getCamera()`: the campaign camera (the original's `WorldCameraBridge` over its
//! camera class, disassembled). Its position is the world point at the
//! centre of the screen. The world state puts it on the player each frame while the
//! camera is locked; clicks and the hover go through `screenToWorld` (and
//! `World.screenToTile`), and the night lights fade in only for places on screen
//! (`isInsideScreen`). See [`Screen`] for the pixel rules.
//!
//! It moves as the original's camera does (see `camera_motion`): `moveTo`,
//! `moveToTile` and `moveToPos` glide there, eased, at 4096 units per second
//! (`moveToPos(x, y, speed)` at that speed, not eased); `jumpTo` and `setPos` are
//! there at once; `move(dx, dy)` shifts it. `zoomTo` and `zoomBy` ease the zoom
//! between 1 / `SceneScale` and 12. The map's bounds (`World.resizeScene`) keep the
//! view on the map. `getPos` rounds to whole units (half away from zero), as the
//! original's does.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{ClassBuilder, Number, Retained, SQVM, SqClass, SqStrongRef, SqTable, Variadic};

use super::camera_motion::{Camera, GLIDE_SPEED};
use super::screen::Screen;

/// What the camera shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraView {
	/// The world point at the centre of the screen.
	pub pos: (f32, f32),
	/// World units per screen pixel.
	pub zoom: f32,
}

impl Default for CameraView {
	fn default() -> Self {
		CameraView {
			pos: (0.0, 0.0),
			zoom: 1.0,
		}
	}
}

/// The camera, shared between its script handle and the engine.
pub(crate) type WorldCamera = Rc<RefCell<Camera>>;

/// What `camera` shows now.
pub(crate) fn view(camera: &WorldCamera) -> CameraView {
	let c = camera.borrow();
	CameraView { pos: c.pos, zoom: c.zoom }
}

/// The furthest the world camera zooms out (`0x4eeaa1`).
const ZOOM_LIMIT: f32 = 12.0;

/// Where the world camera starts (`0x4ee9c8`).
const START: (f32, f32) = (2800.0, 2000.0);

struct CameraHandle(WorldCamera);

/// `createVec(x, y)`.
fn vec(vm: &SQVM, (x, y): (f32, f32)) -> Result<SqStrongRef, String> {
	let create_vec = SqTable::root(vm).get::<SqStrongRef>("createVec").ok_or("createVec missing")?;
	create_vec.call_ret::<SqStrongRef, _>(vm, (f64::from(x), f64::from(y)))?.ok_or_else(|| "createVec returned null".into())
}

/// A vector's `X` and `Y`.
fn xy(vm: &SQVM, v: &SqStrongRef) -> Option<(f32, f32)> {
	let c = |axis: &str| v.get_member::<Number>(vm, axis).map(|n| n.0 as f32);
	Some((c("X")?, c("Y")?))
}

/// A position from `(x, y)` numbers or one vector.
fn pos_arg(vm: &SQVM, args: &[Retained]) -> Option<(f32, f32)> {
	match args {
		[x, y, ..] => {
			let n = |v: &Retained| v.0.get_value::<Number>(vm).map(|n| n.0 as f32);
			Some((n(x)?, n(y)?))
		}
		[v] => xy(vm, &v.0),
		[] => None,
	}
}

/// A pixel from `(x, y)` numbers or one vector, truncated to whole pixels.
fn pixel_arg(vm: &SQVM, args: &[Retained]) -> Option<(i32, i32)> {
	pos_arg(vm, args).map(|(x, y)| (x as i32, y as i32))
}

/// `entity.getPos()` (or a tile's `Pos`): where `moveTo`, `jumpTo` and `moveToTile` go.
fn target_pos(vm: &SQVM, target: &SqStrongRef) -> Option<(f32, f32)> {
	if let Ok(Some(pos)) = target.call_method_ret::<SqStrongRef, _>(vm, "getPos", ()) {
		return xy(vm, &pos);
	}
	target.get_member::<SqStrongRef>(vm, "Pos").and_then(|p| xy(vm, &p))
}

/// The original rounds a float half away from zero to a whole number.
fn round_half_away(v: f32) -> f32 {
	(if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i32 as f32
}

fn build_class(vm: &mut SQVM) -> SqClass<CameraHandle> {
	let set_pos = |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
		if let Some(p) = pos_arg(vm, &a) {
			c.0.borrow_mut().set_pos(p);
		}
	};
	let glide_to = |c: &mut CameraHandle, vm: &SQVM, target: Retained| {
		if let Some(p) = target_pos(vm, &target.0) {
			c.0.borrow_mut().glide(p, GLIDE_SPEED, true, 0.0);
		}
	};
	let mut b = ClassBuilder::<CameraHandle>::new("WorldCamera")
		.property(
			"Zoom",
			|c: &CameraHandle| f64::from(c.0.borrow().zoom),
			|c: &mut CameraHandle, v: f64| c.0.borrow_mut().set_zoom(v as f32),
		)
		.method("getPos", |c: &mut CameraHandle, vm: &SQVM| {
			let (x, y) = c.0.borrow().pos;
			vec(vm, (round_half_away(x), round_half_away(y)))
		})
		.method("setPos", set_pos)
		// (x, y) and (vec) glide eased at the usual speed; (x, y, speed) at that speed.
		.method("moveToPos", move |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let speed = if a.len() == 3 { a[2].0.get_value::<Number>(vm).map(|n| n.0 as f32) } else { None };
			let a = if a.len() == 3 { &a[..2] } else { &a[..] };
			if let Some(p) = pos_arg(vm, a) {
				c.0.borrow_mut().glide(p, speed.unwrap_or(GLIDE_SPEED), speed.is_none(), 0.0);
			}
		})
		// (dx, dy) ends a glide where it is; (vec) shifts its target too.
		.method("move", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let Some(d) = pos_arg(vm, &a) else { return };
			if a.len() == 1 {
				c.0.borrow_mut().shift_with_target(d);
			} else {
				c.0.borrow_mut().shift(d);
			}
		})
		.method("moveTo", glide_to)
		.method("moveToTile", glide_to)
		.method("jumpTo", |c: &mut CameraHandle, vm: &SQVM, target: Retained| {
			if let Some(p) = target_pos(vm, &target.0) {
				c.0.borrow_mut().set_pos(p);
			}
		})
		.method("isMovingToTile", |c: &mut CameraHandle, vm: &SQVM, tile: Retained| {
			target_pos(vm, &tile.0).is_some_and(|p| c.0.borrow().target() == p)
		})
		.method("zoomTo", |c: &mut CameraHandle, zoom: f64, speed: f64| {
			c.0.borrow_mut().zoom_to(zoom as f32, speed as f32)
		})
		.method("zoomBy", |c: &mut CameraHandle, delta: f64| c.0.borrow_mut().zoom_by(delta as f32))
		.method("cancelMovement", |c: &mut CameraHandle| c.0.borrow_mut().cancel())
		.method("screenToWorld", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let (x, y) = pixel_arg(vm, &a).unwrap_or((0, 0));
			let view = view(&c.0);
			vec(vm, Screen::of(vm).to_world(view.pos, view.zoom, x, y))
		})
		.method("worldToScreen", |c: &mut CameraHandle, vm: &SQVM, pos: Retained| {
			let view = view(&c.0);
			let (x, y) = Screen::of(vm).to_screen(view.pos, view.zoom, xy(vm, &pos.0).unwrap_or((0.0, 0.0)));
			vec(vm, (x as f32, y as f32))
		})
		.method("isInsideScreen", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			// (pos, margin); the scripts always pass both.
			let Some(pos) = a.first().and_then(|p| xy(vm, &p.0)) else { return false };
			let margin = a.get(1).and_then(|m| m.0.get_value::<Number>(vm)).map_or(0, |n| n.0 as i32);
			let view = view(&c.0);
			let screen = Screen::of(vm);
			screen.contains(screen.to_screen(view.pos, view.zoom, pos), margin)
		});
	// `update` fires the camera's moved-callbacks (none registered here); the world
	// camera has no levels to climb.
	for name in ["climbBy", "update"] {
		b = b.method(name, |_c: &mut CameraHandle, Variadic(_a): Variadic<Retained>| {});
	}
	b.build(vm)
}

/// The camera instance for `World.getCamera()`, and the camera it drives.
pub(crate) fn instance(vm: &mut SQVM) -> (SqStrongRef, WorldCamera) {
	let class = build_class(vm);
	let mut start = Camera::new((1920, 1080), ZOOM_LIMIT);
	start.shift(START);
	let camera = Rc::new(RefCell::new(start));
	(class.instantiate(vm, CameraHandle(camera.clone())), camera)
}

/// `World.screenToTile(x, y)`: the tile under a pixel, as `World.worldToTile` of the
/// world point there (`0x4ce080`).
pub(crate) fn screen_to_tile(vm: &SQVM, camera: &WorldCamera, x: i32, y: i32) -> Result<Option<SqStrongRef>, String> {
	let view = view(camera);
	let at = vec(vm, Screen::of(vm).to_world(view.pos, view.zoom, x, y))?;
	let world = SqTable::root(vm).get::<SqStrongRef>("World").ok_or("World missing")?;
	world.call_method_ret::<SqStrongRef, _>(vm, "worldToTile", (at,))
}

#[cfg(test)]
mod tests {
	use super::round_half_away;

	#[test]
	fn get_pos_rounds_half_away_from_zero() {
		assert_eq!(round_half_away(1.5), 2.0);
		assert_eq!(round_half_away(1.49), 1.0);
		assert_eq!(round_half_away(-1.5), -2.0);
		assert_eq!(round_half_away(-0.4), 0.0);
	}
}
