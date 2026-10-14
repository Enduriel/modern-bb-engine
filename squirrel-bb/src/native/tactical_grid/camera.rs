//! `Tactical.getCamera()`: the battle camera, headless. It moves instantly, so the AI's
//! waits on it (`agent.IsCameraReady`) always pass: it is never moving, always centred,
//! and everything is on screen.
//!
//! Its `Level` (how many height levels are drawn raised) is logic, not just looks:
//! picking (`Tactical.worldToTile`) treats tiles above it as lower. Scripts set it
//! from `getBestLevelForTile` as the action moves, and the battle starts with
//! `setLevelToHighestOnMap`; both are the original's, disassembled.

use squirrel::{ClassBuilder, Native, Retained, SQVM, SqArray, SqClass, SqStrongRef, SqTable, Variadic};

use super::{Grid, TacticalGridState, TileHandle};

/// The camera's script-visible fields.
pub(crate) struct CameraState {
	pub level: i64,
	pub zoom: f64,
	pub flipped: bool,
	/// The entities with an orientation overlay (the arrow the UI shows for a unit off
	/// screen), by id, in the order they were added.
	pub overlays: Vec<i64>,
}

impl Default for CameraState {
	fn default() -> Self {
		CameraState {
			level: 0,
			zoom: 1.0,
			flipped: false,
			overlays: Vec::new(),
		}
	}
}

/// `entity.getID()`, if it has one.
fn entity_id(vm: &SQVM, entity: &SqStrongRef) -> Option<i64> {
	entity.call_method_ret::<i64, _>(vm, "getID", ()).ok().flatten()
}

/// `queryEntityOverlays()`: what the orientation overlays show, in the UI's fields
/// (`TacticalScreenIdentifier.OrientationOverlay`). Headless, every unit is on screen,
/// so no arrow is visible; the original places those at (0, 0) (the UI golden of a
/// battle, `bb-ui/tests/goldens/world_combat.json`).
fn overlay_states(vm: &SQVM, overlays: &[i64]) -> SqStrongRef {
	let mut list = SqArray::new(vm);
	for &id in overlays {
		let mut overlay = SqTable::new(vm);
		overlay.set("id", id);
		overlay.set("x", 0i64);
		overlay.set("y", 0i64);
		overlay.set("visible", false);
		list.push(overlay.to_strong());
	}
	list.to_strong()
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
		.property("Zoom", |c: &CameraHandle| c.0.borrow().camera.zoom, |c: &mut CameraHandle, v: f64| {
			c.0.borrow_mut().camera.zoom = v;
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
		.method("isCenteredOn", |_c: &mut CameraHandle, Variadic(_a): Variadic<Retained>| true)
		.method("isCenteredOnTile", |_c: &mut CameraHandle, Variadic(_a): Variadic<Retained>| true)
		.method("isInsideScreen", |_c: &mut CameraHandle, Variadic(_a): Variadic<Retained>| true)
		.method("isMoving", |_c: &mut CameraHandle| false)
		.method("isMovingToTile", |_c: &mut CameraHandle, Variadic(_a): Variadic<Retained>| false)
		.method("getPos", |_c: &mut CameraHandle, vm: &SQVM| super::vec(vm, (0.0, 0.0)))
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
	// Moves, zoom, shakes and the overlays' screen positions: visual only.
	for name in [
		"registerCallbacks", "unregisterCallbacks", "quake", "zoomBy", "zoomTo", "move", "moveToExactly",
		"climbBy", "update", "render", "updateEntityOverlays", "cancelMovement", "setPos", "moveTo", "jumpTo",
		"moveToTile", "moveToTileSlowly",
	] {
		b = b.method(name, |_c: &mut CameraHandle, Variadic(_a): Variadic<Retained>| {});
	}
	b.build(vm)
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
}
