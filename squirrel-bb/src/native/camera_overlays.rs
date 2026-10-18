//! The camera class's entity overlays, shared by the world's camera and the battle's:
//! `addEntityOverlay({ entity, image })`, `removeEntityOverlay(entity)`,
//! `removeEntityOverlays()` and `queryEntityOverlays()` (vanilla's battle orientation
//! overlays, orientation_overlay.nut). The overlays' screen positions are visual only
//! (see [`EntityOverlays::states`]).

use squirrel::{SQVM, SqArray, SqStrongRef, SqTable};

/// The overlaid entities' ids, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EntityOverlays(Vec<i64>);

/// An entity's id (`getID()`).
fn entity_id(vm: &SQVM, entity: &SqStrongRef) -> Option<i64> {
	entity.call_method_ret::<i64, _>(vm, "getID", ()).ok().flatten()
}

impl EntityOverlays {
	/// `addEntityOverlay({ entity, image })`, once per entity.
	pub(crate) fn add(&mut self, vm: &SQVM, overlay: &SqStrongRef) {
		let entity = overlay.as_table(vm).and_then(|t| t.get::<SqStrongRef>("entity"));
		if let Some(id) = entity.and_then(|e| entity_id(vm, &e)) {
			if !self.0.contains(&id) {
				self.0.push(id);
			}
		}
	}

	/// `removeEntityOverlay(entity)`.
	pub(crate) fn remove(&mut self, vm: &SQVM, entity: &SqStrongRef) {
		if let Some(id) = entity_id(vm, entity) {
			self.0.retain(|&o| o != id);
		}
	}

	/// `removeEntityOverlays()`.
	pub(crate) fn clear(&mut self) {
		self.0.clear();
	}

	/// `queryEntityOverlays()`: per overlay `{ id, x, y, visible, isOffscreen, yOffset }`,
	/// the UI's fields (`TacticalScreenIdentifier.OrientationOverlay`). Headless, every
	/// unit is on screen, so no arrow is visible; the original places those at (0, 0), not
	/// offscreen and with no vertical offset (the UI golden of a battle,
	/// `bb-ui/tests/goldens/world_combat.json`, and the payload golden
	/// `parity/goldens/ui_payloads/world.txt`).
	pub(crate) fn states(&self, vm: &SQVM) -> SqStrongRef {
		let mut list = SqArray::new(vm);
		for &id in &self.0 {
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
}
