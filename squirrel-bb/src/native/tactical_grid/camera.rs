//! `Tactical.getCamera()`: the battle camera. The host chooses how it moves
//! ([`BattleCamera`]):
//! - Instant (headless, the default): moves arrive at once, so the AI's waits on it
//!   (`agent.IsCameraReady`) always pass: it is never moving, always centred, and
//!   everything is on screen.
//! - Glides (the windowed game): as the original's (its bindings, `0x4bebb0`…, over
//!   the camera class of `camera_motion`): `moveToTile` and `moveTo` glide eased at
//!   4096 units per second, `moveToTileSlowly` at 512, `setPos` and `jumpTo` are there
//!   at once; zooms ease between 1 / `SceneScale` and 4; the map's bounds (from two
//!   tiles beyond it) keep the view on it; `isCenteredOnTile` allows 3 units,
//!   `isCenteredOn` 64. The AI then waits for the camera, as in the original.
//!
//! Its position (what's at the centre of the screen) is real either way, for
//! `screenToWorld` and `Tactical.screenToTile`, which turn clicks into tiles.
//!
//! Its `Level` (how many height levels are drawn raised) is logic, not just looks:
//! picking (`Tactical.worldToTile`) treats tiles above it as lower. Scripts set it
//! from `getBestLevelForTile` as the action moves, and the battle starts with
//! `setLevelToHighestOnMap`; both are the original's, disassembled.

use squirrel::{ClassBuilder, Native, Number, Retained, SQVM, SqArray, SqClass, SqStrongRef, SqTable, Variadic};

use super::{Grid, TacticalGridState, TileHandle};
use crate::native::camera_motion::{Camera, GLIDE_SPEED};
use crate::native::screen::Screen;

/// How the battle camera moves (see the module docs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BattleCamera {
	/// At once: a battle paces the same at any frame rate (draw-exact battles and the
	/// combat goldens depend on it).
	#[default]
	Instant,
	/// As the original's, over real time; the AI waits for its moves.
	Glides,
}

/// The furthest the battle camera zooms out (`0x4ceb1d`).
const ZOOM_LIMIT: f32 = 4.0;

/// `moveToTileSlowly`'s speed (`0x4bec95`).
const SLOW_SPEED: f32 = 512.0;

/// How close counts as centred: on a tile, on an entity (`0x4beaaa`, `0x4beb4e`).
const CENTRED_ON_TILE: f32 = 3.0;
const CENTRED_ON_ENTITY: f32 = 64.0;

/// The camera's script-visible fields.
pub(crate) struct CameraState {
	pub level: i64,
	pub zoom: f64,
	pub flipped: bool,
	/// The world position at the centre of the screen.
	pub pos: (f64, f64),
	/// The entities with an orientation overlay (the arrow the UI shows for a unit off
	/// screen), by id, in the order they were added.
	pub overlays: Vec<i64>,
	/// The gliding camera, with [`BattleCamera::Glides`]: it holds the position and
	/// zoom then.
	pub glide: Option<Camera>,
}

impl CameraState {
	/// What's at the centre of the screen.
	pub fn pos(&self) -> (f64, f64) {
		match &self.glide {
			Some(c) => (f64::from(c.pos.0), f64::from(c.pos.1)),
			None => self.pos,
		}
	}

	/// World units per screen pixel.
	pub fn zoom(&self) -> f64 {
		self.glide.as_ref().map_or(self.zoom, |c| f64::from(c.zoom))
	}

	/// Move this way from now on.
	pub fn set_mode(&mut self, mode: BattleCamera) {
		self.glide = (mode == BattleCamera::Glides).then(|| Camera::new((1920, 1080), ZOOM_LIMIT));
	}

	/// Keep a gliding camera's view on a `width` × `height` map: between the tiles at
	/// axial `(-2, -2)` and `(width + 1, height - width / 2 + 1)` (`0x4cf65d`).
	pub fn bound_to_map(&mut self, width: i64, height: i64) {
		let Some(c) = self.glide.as_mut() else { return };
		let pos = |q: i64, r: i64| {
			let (x, y) = super::tile_pos(q, r + q / 2);
			(x as f32, y as f32)
		};
		c.set_bounds(pos(-2, -2), pos(width + 1, height - width / 2 + 1));
	}
}

impl Default for CameraState {
	fn default() -> Self {
		CameraState {
			level: 0,
			zoom: 1.0,
			flipped: false,
			pos: (0.0, 0.0),
			overlays: Vec::new(),
			glide: None,
		}
	}
}

/// `entity.getID()`, if it has one.
fn entity_id(vm: &SQVM, entity: &SqStrongRef) -> Option<i64> {
	entity.call_method_ret::<i64, _>(vm, "getID", ()).ok().flatten()
}

/// `queryEntityOverlays()`: what the orientation overlays show, in the UI's fields
/// (`TacticalScreenIdentifier.OrientationOverlay`). Headless, every unit is on screen,
/// so no arrow is visible; the original places those at (0, 0), not offscreen and with
/// no vertical offset (the UI golden of a battle, `bb-ui/tests/goldens/world_combat.json`,
/// and the payload golden `parity/goldens/ui_payloads/world.txt`).
fn overlay_states(vm: &SQVM, overlays: &[i64]) -> SqStrongRef {
	let mut list = SqArray::new(vm);
	for &id in overlays {
		let mut overlay = SqTable::new(vm);
		overlay.set("id", id);
		overlay.set("x", 0i64);
		overlay.set("y", 0i64);
		overlay.set("visible", false);
		overlay.set("isOffscreen", false);
		overlay.set("yOffset", 0i64);
		list.push(overlay.to_strong());
	}
	list.to_strong()
}

/// `screenToWorld(x, y)`: the world point under screen pixel `(x, y)`, the camera's
/// position being the centre of the screen: the original's rule (see [`Screen`]),
/// probed at 2560x1440. (The original clamps `Zoom`: 2.0 reads back as 1.640625. Ours
/// doesn't; nothing logic-side zooms.)
pub(super) fn screen_to_world(camera: &CameraState, vm: &SQVM, x: f64, y: f64) -> (f64, f64) {
	let (px, py) = camera.pos();
	let (wx, wy) = Screen::of(vm).to_world((px as f32, py as f32), camera.zoom() as f32, x as i32, y as i32);
	(f64::from(wx), f64::from(wy))
}

/// A position read from a vector or from `entity.getPos()`.
fn pos_of(vm: &SQVM, value: &SqStrongRef) -> Option<(f64, f64)> {
	let v = match value.call_method_ret::<SqStrongRef, _>(vm, "getPos", ()) {
		Ok(Some(p)) => p,
		_ => value.clone(),
	};
	Some((v.get_member::<Number>(vm, "X")?.0, v.get_member::<Number>(vm, "Y")?.0))
}

/// The lowest and highest camera levels.
const MIN_LEVEL: i64 = 0;
const MAX_LEVEL: i64 = 3;

pub(crate) struct CameraHandle(Grid);

/// `setLevelToHighestOnMap()`: the highest tile level on the map (the original scans
/// column by column and stops at 3), within the camera's range.
fn highest_level(g: &TacticalGridState) -> i64 {
	let mut highest = 0;
	'scan: for col in 0..g.width {
		for row in 0..g.height {
			let level = g.tiles[g.idx(col, row).expect("on the map")].level;
			if level > highest {
				highest = level;
				if highest == 3 {
					break 'scan;
				}
			}
		}
	}
	highest.clamp(MIN_LEVEL, MAX_LEVEL)
}

/// `getBestLevelForTile(tile)` with the camera at `current`: at least the tile's level
/// (or the level of what stands on it), at most 3 above. If something stands there
/// and the tile in front of it (the row below) is 2+ levels higher, exactly its level,
/// so the rise in front is drawn flat and doesn't hide it.
fn best_level(g: &TacticalGridState, id: usize, current: i64) -> i64 {
	let t = &g.tiles[id];
	let level = t.level;
	if current < level {
		return level;
	}
	let hidden = t.occupant.is_some()
		&& g.idx_axial(t.q, t.r - 1).is_some_and(|front| g.tiles[front].level - level >= 2);
	if hidden {
		return level;
	}
	current.min(level + 3)
}

impl TacticalGridState {
	/// `entity.isOccludedByTerrain()` for a unit on `tile`: the tile in front of it (the
	/// row below) is 2+ levels higher, so it would be drawn over the unit; never with the
	/// camera flipped (disassembled, 0x4c3030).
	pub(crate) fn occluded_by_terrain(&self, tile: usize) -> bool {
		if self.camera.flipped {
			return false;
		}
		let t = &self.tiles[tile];
		self.idx_axial(t.q, t.r - 1).is_some_and(|front| self.tiles[front].level - t.level >= 2)
	}
}

/// Whether the player lets the camera follow height (`AdjustCameraLevel`); without it
/// `getBestLevelForTile` keeps the current level.
fn adjusts_level(vm: &SQVM) -> bool {
	SqTable::root(vm)
		.get::<SqTable>("Settings")
		.and_then(|s| s.get::<SqStrongRef>("getGameplaySettings"))
		.and_then(|f| f.call_ret::<SqStrongRef, _>(vm, ()).ok().flatten())
		.and_then(|t| t.as_table(vm))
		.and_then(|t| t.get::<bool>("AdjustCameraLevel"))
		.unwrap_or(true)
}

pub(super) fn build_class(vm: &mut SQVM) -> SqClass<CameraHandle> {
	let mut b = ClassBuilder::<CameraHandle>::new("TacticalCamera")
		.property("Level", |c: &CameraHandle| c.0.borrow().camera.level, |c: &mut CameraHandle, v: i64| {
			c.0.borrow_mut().camera.level = v;
		})
		.property("Zoom", |c: &CameraHandle| c.0.borrow().camera.zoom(), |c: &mut CameraHandle, v: f64| {
			let camera = &mut c.0.borrow_mut().camera;
			match camera.glide.as_mut() {
				Some(g) => g.set_zoom(v as f32),
				None => camera.zoom = v,
			}
		})
		.property("IsFlipped", |c: &CameraHandle| c.0.borrow().camera.flipped, |c: &mut CameraHandle, v: bool| {
			c.0.borrow_mut().camera.flipped = v;
		})
		.method("setLevelToHighestOnMap", |c: &mut CameraHandle| {
			let level = highest_level(&c.0.borrow());
			c.0.borrow_mut().camera.level = level;
		})
		.method("getBestLevelForTile", |c: &mut CameraHandle, vm: &SQVM, tile: Native<TileHandle>| -> i64 {
			let g = c.0.borrow();
			let current = g.camera.level;
			if !adjusts_level(vm) {
				return current;
			}
			best_level(&g, tile.0.id, current)
		})
		// Instant, it's always where it's asked to be; gliding, as the original.
		.method("isCenteredOn", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let target = a.first().and_then(|e| pos_of(vm, &e.0));
			centred(&c.0.borrow().camera, target, CENTRED_ON_ENTITY)
		})
		.method("isCenteredOnTile", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let g = c.0.borrow();
			let target = a.first().and_then(|t| tile_pos_of(&g, vm, t));
			centred(&g.camera, target, CENTRED_ON_TILE)
		})
		.method("isInsideScreen", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let g = c.0.borrow();
			if g.camera.glide.is_none() {
				return true;
			}
			let Some(pos) = a.first().and_then(|p| pos_of(vm, &p.0)) else { return false };
			let margin = a.get(1).and_then(|m| m.0.get_value::<Number>(vm)).map_or(0, |n| n.0 as i32);
			let (cx, cy) = g.camera.pos();
			let screen = Screen::of(vm);
			let pixel = screen.to_screen((cx as f32, cy as f32), g.camera.zoom() as f32, (pos.0 as f32, pos.1 as f32));
			screen.contains(pixel, margin)
		})
		.method("isMoving", |c: &mut CameraHandle| c.0.borrow().camera.glide.as_ref().is_some_and(Camera::is_moving))
		.method("isMovingToTile", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let g = c.0.borrow();
			let Some(glide) = g.camera.glide.as_ref() else { return false };
			a.first().and_then(|t| tile_pos_of(&g, vm, t)).is_some_and(|p| glide.target() == (p.0 as f32, p.1 as f32))
		})
		.method("getPos", |c: &mut CameraHandle, vm: &SQVM| super::vec(vm, c.0.borrow().camera.pos()))
		// To a position, a tile or an entity: at once, or gliding (eased; a second
		// argument is how close is close enough).
		.method("setPos", |c: &mut CameraHandle, vm: &SQVM, pos: Retained| {
			if let Some(p) = pos_of(vm, &pos.0) {
				jump(&mut c.0.borrow_mut().camera, p);
			}
		})
		.method("moveToTile", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let p = a.first().and_then(|t| tile_pos_of(&c.0.borrow(), vm, t));
			if let Some(p) = p {
				glide(&mut c.0.borrow_mut().camera, p, GLIDE_SPEED, radius(vm, &a));
			}
		})
		.method("moveToTileSlowly", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let p = a.first().and_then(|t| tile_pos_of(&c.0.borrow(), vm, t));
			if let Some(p) = p {
				glide(&mut c.0.borrow_mut().camera, p, SLOW_SPEED, radius(vm, &a));
			}
		})
		.method("screenToWorld", |c: &mut CameraHandle, vm: &SQVM, x: Number, y: Number| {
			let p = screen_to_world(&c.0.borrow().camera, vm, x.0, y.0);
			super::vec(vm, p)
		})
		// The orientation overlays (orientation_overlay.nut): `addEntityOverlay` gets
		// `{ entity, image }`, `removeEntityOverlay` the entity.
		.method("addEntityOverlay", |c: &mut CameraHandle, vm: &SQVM, overlay: Retained| {
			let entity = overlay.0.as_table(vm).and_then(|t| t.get::<SqStrongRef>("entity"));
			if let Some(id) = entity.and_then(|e| entity_id(vm, &e)) {
				let overlays = &mut c.0.borrow_mut().camera.overlays;
				if !overlays.contains(&id) {
					overlays.push(id);
				}
			}
		})
		.method("removeEntityOverlay", |c: &mut CameraHandle, vm: &SQVM, entity: Retained| {
			if let Some(id) = entity_id(vm, &entity.0) {
				c.0.borrow_mut().camera.overlays.retain(|&o| o != id);
			}
		})
		.method("removeEntityOverlays", |c: &mut CameraHandle| {
			c.0.borrow_mut().camera.overlays.clear();
		})
		.method("queryEntityOverlays", |c: &mut CameraHandle, vm: &SQVM, Variadic(_a): Variadic<Retained>| {
			let overlays = c.0.borrow().camera.overlays.clone();
			overlay_states(vm, &overlays)
		});
	b = b.method("jumpTo", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
		if let Some(p) = a.first().and_then(|e| pos_of(vm, &e.0)) {
			jump(&mut c.0.borrow_mut().camera, p);
		}
	});
	for name in ["moveTo", "moveToExactly"] {
		b = b.method(name, |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			if let Some(p) = a.first().and_then(|e| pos_of(vm, &e.0)) {
				glide(&mut c.0.borrow_mut().camera, p, GLIDE_SPEED, radius(vm, &a));
			}
		});
	}
	// Zooming and panning ease or shift only a gliding camera; instant, they're looks.
	b = b
		.method("zoomTo", |c: &mut CameraHandle, zoom: f64, speed: f64| {
			if let Some(g) = c.0.borrow_mut().camera.glide.as_mut() {
				g.zoom_to(zoom as f32, speed as f32);
			}
		})
		.method("zoomBy", |c: &mut CameraHandle, delta: f64| {
			if let Some(g) = c.0.borrow_mut().camera.glide.as_mut() {
				g.zoom_by(delta as f32);
			}
		})
		.method("move", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
			let mut g = c.0.borrow_mut();
			let Some(glide) = g.camera.glide.as_mut() else { return };
			let n = |v: &Retained| v.0.get_value::<Number>(vm).map(|n| n.0 as f32);
			match &a[..] {
				[x, y, ..] => {
					if let (Some(x), Some(y)) = (n(x), n(y)) {
						glide.shift((x, y));
					}
				}
				[v] => {
					if let Some((x, y)) = pos_of(vm, &v.0) {
						glide.shift_with_target((x as f32, y as f32));
					}
				}
				[] => {}
			}
		})
		.method("cancelMovement", |c: &mut CameraHandle| {
			if let Some(g) = c.0.borrow_mut().camera.glide.as_mut() {
				g.cancel();
			}
		});
	// quake(from, to, strength, out, back) along the way from one entity to the other,
	// or quake(direction, strength, out, back): a gliding camera shakes.
	b = b.method("quake", |c: &mut CameraHandle, vm: &SQVM, Variadic(a): Variadic<Retained>| {
		let mut g = c.0.borrow_mut();
		let Some(glide) = g.camera.glide.as_mut() else { return };
		let n = |i: usize| a.get(i).and_then(|v| v.0.get_value::<Number>(vm)).map_or(0.0, |n| n.0 as f32);
		let now = crate::native::sprite_color::virtual_time(vm);
		if a.len() >= 5 {
			let (Some(from), Some(to)) = (pos_of(vm, &a[0].0), pos_of(vm, &a[1].0)) else { return };
			let (dx, dy) = ((to.0 - from.0) as f32, (to.1 - from.1) as f32);
			if dx == 0.0 && dy == 0.0 {
				return;
			}
			let inv = 1.0 / f64::from(dy * dy + dx * dx).sqrt() as f32;
			glide.quake((inv * dx, inv * dy), n(2), n(3), n(4), now);
		} else if let Some(dir) = a.first().and_then(|v| pos_of(vm, &v.0)) {
			glide.quake((dir.0 as f32, dir.1 as f32), n(1), n(2), n(3), now);
		}
	});
	// The level's easing and the overlays' screen positions: visual only.
	for name in ["registerCallbacks", "unregisterCallbacks", "climbBy", "update", "render", "updateEntityOverlays"] {
		let stub = crate::native::stubs::name(format!("Tactical.getCamera().{name}"));
		b = b.method(name, move |_c: &mut CameraHandle, Variadic(_a): Variadic<Retained>| crate::native::stubs::note(stub));
	}
	b.build(vm)
}

/// `tile.Pos` for a tile argument.
fn tile_pos_of(g: &TacticalGridState, vm: &SQVM, tile: &Retained) -> Option<(f64, f64)> {
	let id = g.class.as_ref()?.with_payload(vm, &tile.0, |t| t.id)?;
	g.tiles.get(id).map(|t| t.world_pos())
}

/// A glide's second argument: how close to stop (0 to land on the target).
fn radius(vm: &SQVM, args: &[Retained]) -> f32 {
	args.get(1).and_then(|r| r.0.get_value::<Number>(vm)).map_or(0.0, |n| n.0 as f32)
}

/// Whether the camera is centred on `target` within `tolerance`; instant, always.
fn centred(camera: &CameraState, target: Option<(f64, f64)>, tolerance: f32) -> bool {
	match (&camera.glide, target) {
		(None, _) => true,
		(Some(g), Some(p)) => g.is_centered_on((p.0 as f32, p.1 as f32), tolerance),
		(Some(_), None) => false,
	}
}

/// Put the camera on `p` at once.
fn jump(camera: &mut CameraState, p: (f64, f64)) {
	match camera.glide.as_mut() {
		Some(g) => g.set_pos((p.0 as f32, p.1 as f32)),
		None => camera.pos = p,
	}
}

/// Glide to `p` at `speed`, eased; instant, jump.
fn glide(camera: &mut CameraState, p: (f64, f64), speed: f32, radius: f32) {
	match camera.glide.as_mut() {
		Some(g) => g.glide((p.0 as f32, p.1 as f32), speed, true, radius),
		None => camera.pos = p,
	}
}

/// The one camera instance (for `Tactical.getCamera`).
pub(super) fn instance(vm: &SQVM, class: &SqClass<CameraHandle>, grid: &Grid) -> SqStrongRef {
	class.instantiate(vm, CameraHandle(grid.clone()))
}

#[cfg(test)]
mod tests {
	use super::{TacticalGridState, best_level, highest_level};

	#[test]
	fn levels_follow_the_map() {
		let mut g = TacticalGridState::for_test(8, 8);
		assert_eq!(highest_level(&g), 0);
		let a = g.idx(3, 3).unwrap();
		g.tiles[a].level = 2;
		assert_eq!(highest_level(&g), 2);
		// Empty tile: raised to it, else kept.
		assert_eq!(best_level(&g, a, 0), 2);
		assert_eq!(best_level(&g, a, 3), 3);
		let low = g.idx(3, 4).unwrap();
		assert_eq!(best_level(&g, low, 1), 1);
	}

	#[test]
	fn a_unit_is_occluded_by_terrain_two_levels_higher_in_front() {
		let mut g = TacticalGridState::for_test(8, 8);
		let unit = g.idx(3, 4).unwrap();
		let (q, r) = (g.tiles[unit].q, g.tiles[unit].r);
		let front = g.idx_axial(q, r - 1).unwrap();
		g.tiles[front].level = 1;
		assert!(!g.occluded_by_terrain(unit));
		g.tiles[front].level = 2;
		assert!(g.occluded_by_terrain(unit));
		g.camera.flipped = true;
		assert!(!g.occluded_by_terrain(unit));
	}
}
