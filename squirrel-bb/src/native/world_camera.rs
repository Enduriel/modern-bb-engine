//! `World.getCamera()`: the campaign camera (the original's `WorldCameraBridge` over its
//! camera class, disassembled). Its position is the world point at the
//! centre of the screen. The world state puts it on the player each frame while the
//! camera is locked; clicks and the hover go through `screenToWorld` (and
//! `World.screenToTile`), and the night lights fade in only for places on screen
//! (`isInsideScreen`). See [`Screen`] for the pixel rules.
//!
//! The original's camera glides: `moveTo`, `moveToPos` and `moveToTile` set a target it
//! eases towards in its update. Headless, as the battle camera, it arrives at once.
//! `move(dx, dy)` shifts it by world units, as in the original. `getPos` rounds to whole
//! units (half away from zero), as the original's does.
//!
//! Not yet: the bounds `setPos` keeps the view inside when they're on, zoom easing
//! (`zoomTo`, `zoomBy`; `Zoom` keeps what scripts set), and `climbBy`.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{ClassBuilder, Number, Retained, SQVM, SqClass, SqStrongRef, SqTable, Variadic};

use super::screen::Screen;

/// What the camera shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CameraView {
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
pub(crate) type WorldCamera = Rc<RefCell<CameraView>>;

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
			c.0.borrow_mut().pos = p;
		}
	};
	let to_target = |c: &mut CameraHandle, vm: &SQVM, target: Retained| {
		if let Some(p) = target_pos(vm, &target.0) {
			c.0.borrow_mut().pos = p;
		}
	};
	let mut b = ClassBuilder::<CameraHandle>::new("WorldCamera")
		.property(
			"Zoom",
			|c: &CameraHandle| f64::from(c.0.borrow().zoom),
			|c: &mut CameraHandle, v: f64| c.0.borrow_mut().zoom = v as f32,
		)
		.method("getPos", |c: &mut CameraHandle, vm: &SQVM| {
			let (x, y) = c.0.borrow().pos;
			vec(vm, (round_half_away(x), round_half_away(y)))
		})
		.method("setPos", set_pos)
		.method("moveToPos", move |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			// (x, y[, speed]) or (vec): the speed only paces the glide.
			let a = if a.len() == 3 { &a[..2] } else { &a[..] };
			if let Some(p) = pos_arg(vm, a) {
				c.0.borrow_mut().pos = p;
			}
		})
		.method("move", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			if let Some((dx, dy)) = pos_arg(vm, &a) {
				let mut view = c.0.borrow_mut();
				view.pos = (view.pos.0 + dx, view.pos.1 + dy);
			}
		})
		.method("moveTo", to_target)
		.method("jumpTo", to_target)
		.method("moveToTile", to_target)
		.method("isMovingToTile", |_c: &mut CameraHandle, Variadic(_a): Variadic<Retained>| false)
		.method("screenToWorld", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let (x, y) = pixel_arg(vm, &a).unwrap_or((0, 0));
			let view = *c.0.borrow();
			vec(vm, Screen::of(vm).to_world(view.pos, view.zoom, x, y))
		})
		.method("worldToScreen", |c: &mut CameraHandle, vm: &SQVM, pos: Retained| {
			let view = *c.0.borrow();
			let (x, y) = Screen::of(vm).to_screen(view.pos, view.zoom, xy(vm, &pos.0).unwrap_or((0.0, 0.0)));
			vec(vm, (x as f32, y as f32))
		})
		.method("isInsideScreen", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			// (pos, margin); the scripts always pass both.
			let Some(pos) = a.first().and_then(|p| xy(vm, &p.0)) else { return false };
			let margin = a.get(1).and_then(|m| m.0.get_value::<Number>(vm)).map_or(0, |n| n.0 as i32);
			let view = *c.0.borrow();
			let screen = Screen::of(vm);
			screen.contains(screen.to_screen(view.pos, view.zoom, pos), margin)
		});
	for name in ["zoomTo", "zoomBy", "climbBy", "update", "cancelMovement"] {
		b = b.method(name, |_c: &mut CameraHandle, Variadic(_a): Variadic<Retained>| {});
	}
	b.build(vm)
}

/// The camera instance for `World.getCamera()`, and the camera it drives.
pub(crate) fn instance(vm: &mut SQVM) -> (SqStrongRef, WorldCamera) {
	let class = build_class(vm);
	let camera = WorldCamera::default();
	(class.instantiate(vm, CameraHandle(camera.clone())), camera)
}

/// `World.screenToTile(x, y)`: the tile under a pixel, as `World.worldToTile` of the
/// world point there (`0x4ce080`).
pub(crate) fn screen_to_tile(vm: &SQVM, camera: &WorldCamera, x: i32, y: i32) -> Result<Option<SqStrongRef>, String> {
	let view = *camera.borrow();
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
