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

use squirrel::{ClassBuilder, Number, Retained, SQVM, SqClass, SqStrongRef, SqTable, This, Variadic};

use super::camera_callbacks::{self, MovementCallbacks};
use super::camera_overlays::EntityOverlays;
use super::overload::class_overloads;

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

/// How near `isCenteredOnPos` takes as centred (`0x63d320`).
const CENTRED_ON_POS: f32 = 3.0;

/// Where the world camera starts (`0x4ee9c8`).
const START: (f32, f32) = (2800.0, 2000.0);

/// The camera, its `Level` (the camera class's, shared with the battle's: the world map
/// has no levels, so nothing reads it; 0 at first, probed), and its entity
/// overlays.
struct CameraHandle(WorldCamera, i64, EntityOverlays);

/// The world camera's movement callbacks (see `camera_callbacks`). They hold script
/// functions: the engine clears them before the VM closes.
pub(crate) type WorldCameraCallbacks = Rc<RefCell<MovementCallbacks>>;

/// `createVec(x, y)`.
fn vec(vm: &SQVM, (x, y): (f32, f32)) -> Result<SqStrongRef, String> {
	let create_vec = SqTable::root(vm).get::<SqStrongRef>("createVec").ok_or("createVec missing")?;
	create_vec.call_ret::<SqStrongRef, _>(vm, (f64::from(x), f64::from(y)))?.ok_or_else(|| "createVec returned null".into())
}

/// The `vec2f` handed to `native` (see [`super::vec::position`]).
fn xy(vm: &SQVM, v: &SqStrongRef, native: &str) -> Result<(f32, f32), String> {
	super::vec::position_f32(vm, v, native)
}

/// A position from `(x, y)` numbers or one `vec2f`.
fn pos_arg(vm: &SQVM, args: &[Retained], native: &str) -> Result<Option<(f32, f32)>, String> {
	match args {
		[x, y, ..] => {
			let n = |v: &Retained| v.0.get_value::<Number>(vm).map(|n| n.0 as f32);
			Ok(n(x).zip(n(y)))
		}
		[v] => xy(vm, &v.0, native).map(Some),
		[] => Ok(None),
	}
}

/// A pixel from `(x, y)` numbers or one `vec2f`, truncated to whole pixels.
fn pixel_arg(vm: &SQVM, args: &[Retained], native: &str) -> Result<Option<(i32, i32)>, String> {
	Ok(pos_arg(vm, args, native)?.map(|(x, y)| (x as i32, y as i32)))
}

/// `entity.getPos()` (or a tile's `Pos`): where `moveTo`, `jumpTo` and `moveToTile` go.
fn target_pos(vm: &SQVM, target: &SqStrongRef, native: &str) -> Result<Option<(f32, f32)>, String> {
	let pos = match target.call_method_ret::<SqStrongRef, _>(vm, "getPos", ()) {
		Ok(Some(pos)) => Some(pos),
		_ => target.get_member::<SqStrongRef>(vm, "Pos"),
	};
	pos.map(|p| xy(vm, &p, native)).transpose()
}

/// The original rounds a float half away from zero to a whole number.
fn round_half_away(v: f32) -> f32 {
	(if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i32 as f32
}

fn build_class(vm: &mut SQVM, camera: &WorldCamera, callbacks: &WorldCameraCallbacks) -> SqClass<CameraHandle> {
	let glide_to = |c: &mut CameraHandle, vm: &SQVM, target: Retained| -> Result<(), String> {
		if let Some(p) = target_pos(vm, &target.0, "moveTo")? {
			c.0.borrow_mut().glide(p, GLIDE_SPEED, true, 0.0);
		}
		Ok(())
	};
	let mut b = ClassBuilder::<CameraHandle>::new("WorldCamera")
		// The world's camera has no levels: its range is 0 to 0 (disassembled, 0x4eeb31),
		// so setting `Level` or climbing (`climbBy`, the battle camera's 0x4be4c0) keeps 0.
		.property("Level", |c: &CameraHandle| c.1, |c: &mut CameraHandle, v: i64| c.1 = v.clamp(0, 0))
		.method("climbBy", |c: &mut CameraHandle, n: i64| c.1 = c.1.saturating_add(n).clamp(0, 0))
		// Never mirrored on the world map; read only, as the class's.
		.property_ro("IsFlipped", |_c: &CameraHandle| false)
		.property(
			"Zoom",
			|c: &CameraHandle| f64::from(c.0.borrow().zoom),
			|c: &mut CameraHandle, v: f64| c.0.borrow_mut().set_zoom(v as f32),
		)
		.method("getPos", |c: &mut CameraHandle, vm: &SQVM| {
			let (x, y) = c.0.borrow().pos;
			vec(vm, (round_half_away(x), round_half_away(y)))
		})
		.method("moveTo", glide_to)
		.method("moveToTile", glide_to)
		.method("jumpTo", |c: &mut CameraHandle, vm: &SQVM, target: Retained| -> Result<(), String> {
			if let Some(p) = target_pos(vm, &target.0, "jumpTo")? {
				c.0.borrow_mut().set_pos(p);
			}
			Ok(())
		})
		.method("isMovingToTile", |c: &mut CameraHandle, vm: &SQVM, tile: Retained| -> Result<bool, String> {
			Ok(target_pos(vm, &tile.0, "isMovingToTile")?.is_some_and(|p| c.0.borrow().target() == p))
		})
		.method("zoomTo", |c: &mut CameraHandle, zoom: f64, speed: f64| {
			c.0.borrow_mut().zoom_to(zoom as f32, speed as f32)
		})
		.method("zoomBy", |c: &mut CameraHandle, delta: f64| c.0.borrow_mut().zoom_by(delta as f32))
		.method("cancelMovement", |c: &mut CameraHandle| c.0.borrow_mut().cancel())
		.method("worldToScreen", |c: &mut CameraHandle, vm: &SQVM, pos: Retained| {
			let view = view(&c.0);
			let (x, y) = Screen::of(vm).to_screen(view.pos, view.zoom, xy(vm, &pos.0, "worldToScreen")?);
			vec(vm, (x as f32, y as f32))
		})
		// Within 3 units of `pos` on both axes (`0x4a3ce0`, the camera class's).
		.method("isCenteredOnPos", |c: &mut CameraHandle, vm: &SQVM, pos: Retained| -> Result<bool, String> {
			Ok(c.0.borrow().is_centered_on(xy(vm, &pos.0, "isCenteredOnPos")?, CENTRED_ON_POS))
		});
	// Sqrat overloads, as the original's: the arities it binds.
	// (vec) or (x, y): there at once.
	b = class_overloads!(b, CameraHandle, "setPos", [1, 2], |c: &mut CameraHandle, vm: &SQVM, a: &[Retained]| {
		if let Some(p) = pos_arg(vm, a, "setPos")? {
			c.0.borrow_mut().set_pos(p);
		}
		Ok::<(), String>(())
	});
	// (vec) and (x, y) glide eased at the usual speed; (x, y, speed) at that speed.
	b = class_overloads!(b, CameraHandle, "moveToPos", [1, 2, 3], |c: &mut CameraHandle, vm: &SQVM, a: &[Retained]| {
		let speed = if a.len() == 3 { a[2].0.get_value::<Number>(vm).map(|n| n.0 as f32) } else { None };
		let a = if a.len() == 3 { &a[..2] } else { a };
		if let Some(p) = pos_arg(vm, a, "moveToPos")? {
			c.0.borrow_mut().glide(p, speed.unwrap_or(GLIDE_SPEED), speed.is_none(), 0.0);
		}
		Ok::<(), String>(())
	});
	// (dx, dy) ends a glide where it is; (vec) shifts its target too.
	b = class_overloads!(b, CameraHandle, "move", [1, 2], |c: &mut CameraHandle, vm: &SQVM, a: &[Retained]| {
		if let Some(d) = pos_arg(vm, a, "move")? {
			if a.len() == 1 {
				c.0.borrow_mut().shift_with_target(d);
			} else {
				c.0.borrow_mut().shift(d);
			}
		}
		Ok::<(), String>(())
	});
	// (vec) or (x, y): the world point under a pixel.
	b = class_overloads!(b, CameraHandle, "screenToWorld", [1, 2], |c: &mut CameraHandle, vm: &SQVM, a: &[Retained]| {
		let (x, y) = pixel_arg(vm, a, "screenToWorld")?.unwrap_or((0, 0));
		let view = view(&c.0);
		vec(vm, Screen::of(vm).to_world(view.pos, view.zoom, x, y))
	});
	// (pos) or (pos, margin): whether it's on screen.
	b = class_overloads!(b, CameraHandle, "isInsideScreen", [1, 2], |c: &mut CameraHandle, vm: &SQVM, a: &[Retained]| {
		let Some(pos) = a.first().map(|p| xy(vm, &p.0, "isInsideScreen")).transpose()? else { return Ok::<bool, String>(false) };
		let margin = a.get(1).and_then(|m| m.0.get_value::<Number>(vm)).map_or(0, |n| n.0 as i32);
		let view = view(&c.0);
		let screen = Screen::of(vm);
		Ok(screen.contains(screen.to_screen(view.pos, view.zoom, pos), margin))
	});
	// `update` fires the camera's moved-callbacks (none registered here); the world
	// camera has no levels to climb.
	// The camera class's entity overlays (see `camera_overlays`); vanilla uses them only
	// in battles.
	b = b
		.method("addEntityOverlay", |c: &mut CameraHandle, vm: &SQVM, overlay: Retained| c.2.add(vm, &overlay.0, |_| Some((0, 0))))
		.method("removeEntityOverlay", |c: &mut CameraHandle, vm: &SQVM, entity: Retained| c.2.remove(vm, &entity.0))
		.method("removeEntityOverlays", |c: &mut CameraHandle| c.2.clear())
		.method("queryEntityOverlays", |c: &mut CameraHandle, vm: &SQVM, Variadic(_a): Variadic<Retained>| c.2.states(vm));
	// Its movement callbacks (see `camera_callbacks`). They run scripts, so they reach the
	// lists through `callbacks` rather than the instance's payload.
	let (register, unregister, update) = (callbacks.clone(), callbacks.clone(), callbacks.clone());
	let moving = camera.clone();
	b = b
		.native_method("registerCallbacks", move |vm: &SQVM, this: This, start: Retained, end: Retained| {
			register.borrow_mut().register(this.to_strong(vm), start.0, end.0)
		})
		.native_method("unregisterCallbacks", move |start: Retained, end: Retained| unregister.borrow_mut().unregister(&start.0, &end.0))
		.native_method("update", move |vm: &SQVM| {
			let due = update.borrow_mut().due(moving.borrow().is_moving());
			camera_callbacks::fire(vm, due);
		});
	// The overlays' screen positions are visual.
	for name in ["render", "updateEntityOverlays"] {
		let stub = super::stubs::name(format!("World.getCamera().{name}"));
		b = b.method(name, move |_c: &mut CameraHandle, Variadic(_a): Variadic<Retained>| super::stubs::note(stub));
	}
	b.build(vm)
}

/// The camera instance for `World.getCamera()`, the camera it drives, and its movement
/// callbacks.
pub(crate) fn instance(vm: &mut SQVM) -> (SqStrongRef, WorldCamera, WorldCameraCallbacks) {
	let callbacks = WorldCameraCallbacks::default();
	let mut start = Camera::new((1920, 1080), ZOOM_LIMIT);
	start.shift(START);
	let camera = Rc::new(RefCell::new(start));
	let class = build_class(vm, &camera, &callbacks);
	(class.instantiate(vm, CameraHandle(camera.clone(), 0, EntityOverlays::default())), camera, callbacks)
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
