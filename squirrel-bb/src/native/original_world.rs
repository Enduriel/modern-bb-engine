//! The engine's parts as the original's save layout needs them (see
//! `save::original::world::OriginalWorld`).

use squirrel::{SQVM, SqStrongRef};

use super::TacticalEntities;
use super::WorldEntities;
use super::save::original::camera::{Camera, Clock};
use super::save::original::record::{Head, TacticalNative, WorldEntityPart};
use super::save::original::scene::Scene;
use super::save::original::sprite::Sprite;
use super::save::original::world::{OriginalWorld, Roster};
use super::time::TimeService;
use super::world_entities::argb;
use super::world_grid::WorldGrid;
use super::{world_camera, world_time};

pub(crate) struct EngineWorld {
	pub grid: WorldGrid,
	pub entities: WorldEntities,
	pub units: TacticalEntities,
	pub time: TimeService,
	pub clock: world_time::Clock,
	pub camera: world_camera::WorldCamera,
}

impl OriginalWorld for EngineWorld {
	fn clear(&self, _vm: &SQVM) -> Result<(), String> {
		self.units.clear_for_load();
		self.entities.clear_for_load();
		Ok(())
	}

	fn set_virtual_time(&self, seconds: f64) {
		self.time.set_virtual_seconds(seconds);
	}

	fn load_map(&self, _vm: &SQVM, scene: &Scene) -> Result<(), String> {
		self.grid.load_original(scene)?;
		// The scene keeps the lighting; the clock has it here.
		let mut clock = self.clock.borrow_mut();
		let (time, into_phase, phase) = clock.saved();
		clock.load_original(time, into_phase, phase, argb(scene.ambient_color), scene.ambient_saturation);
		Ok(())
	}

	fn load_entity(&self, vm: &SQVM, path: &str, top: bool, head: &Head, part: &WorldEntityPart) -> Result<(), String> {
		self.entities.load_original(vm, path, top, head, part, self.grid.dims())
	}

	fn load_footprint(&self, head: &Head, sprite: &Sprite, expires: f32, kind: u16) {
		self.entities.load_original_footprint(head, sprite, expires, kind);
	}

	fn load_camera(&self, camera: &Camera) {
		let mut c = self.camera.borrow_mut();
		c.set_zoom(camera.zoom);
		c.set_pos(camera.pos);
	}

	fn load_clock(&self, saved: &Clock) {
		let mut clock = self.clock.borrow_mut();
		let (ambient, saturation) = clock.ambient();
		clock.load_original(saved.time, saved.phase_seconds, saved.phase, ambient, saturation);
	}

	fn load_unit(&self, vm: &SQVM, path: &str, head: &Head, native: &TacticalNative) -> Result<SqStrongRef, String> {
		self.units.load_original(vm, path, head, native)
	}

	fn add_unit(&self, roster: Roster, unit: SqStrongRef) {
		self.units.add_loaded(roster, unit);
	}

	fn entity_scripts(&self) -> Vec<(u32, SqStrongRef)> {
		self.entities.scripts_by_id_descending()
	}

	fn load_fog(&self, mask: Vec<u8>) -> Result<(), String> {
		self.grid.load_original_fog(mask)
	}
}
