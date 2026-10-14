//! The engine's parts as the original's save layout needs them (see
//! `save::original::world::OriginalWorld`).

use squirrel::{SQVM, SqStrongRef};

use super::TacticalEntities;
use super::WorldEntities;
use super::save::original::camera::{Camera, Clock};
use super::save::original::record::{Head, TacticalNative, WorldEntityPart, type_id};
use super::save::original::scene::{Layer, LayerBody, QuadTreeLayer, Scene};
use super::save::original::sprite::Sprite;
use super::save::original::world::{OriginalWorld, Roster, Rosters};
use super::time::TimeService;
use super::world_entities::{argb, rgba};
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

	fn create_roster(&self, roster: Roster) {
		self.units.ensure_roster(roster);
	}

	fn entity_scripts(&self) -> Vec<(u32, SqStrongRef)> {
		self.entities.scripts_by_id_descending()
	}

	fn load_fog(&self, mask: Vec<u8>) -> Result<(), String> {
		self.grid.load_original_fog(mask)
	}

	fn virtual_time(&self) -> f64 {
		self.time.virtual_seconds()
	}

	fn scene(&self, vm: &SQVM) -> Result<Scene, String> {
		let (grid, tiles, mut detail) = self.grid.original_map();
		let (top, rest) = self.entities.original_entities(vm)?;
		detail.extend(rest);
		detail.extend(self.entities.original_footprints());
		// The original lists a layer's entities in its quadtree's order, which it doesn't
		// keep across a load either; by id here.
		detail.sort_by_key(|e| e.head.id);
		let (ambient, saturation) = self.clock.borrow().ambient();
		// The quadtrees span the map: the corner past its last tile.
		let bounds = super::world_layout::tile_pos(i64::from(grid.width), i64::from(grid.height));
		let quadtree = |name: &str, u2c: u32, flags: u32, entities: Vec<_>| Layer {
			u2c: Some(u2c),
			name: name.as_bytes().to_vec(),
			type_: type_id("QuadTreeEntityLayer"),
			flags,
			body: Some(LayerBody::QuadTree(QuadTreeLayer {
				depth: 8,
				bounds: [0.0, bounds.0 as f32, 0.0, bounds.1 as f32],
				entities,
			})),
		};
		Ok(Scene {
			d4: 41,
			name: Vec::new(),
			ambient_color: rgba(ambient),
			ambient_saturation: saturation,
			b8: 0,
			zeros: [0; 3],
			r28: [0, 0, 0, 0, 2, 0, 0, 0],
			grid: Some(grid),
			e0: 1,
			layers: vec![
				Layer {
					u2c: Some(0),
					name: b"tiles".to_vec(),
					type_: type_id("HexTileLayer"),
					flags: 13,
					body: Some(LayerBody::Tiles(tiles)),
				},
				quadtree("detail", 0, 13, detail),
				quadtree("entity", 1, 13, top),
				quadtree("effect", 1, 5, Vec::new()),
			],
		})
	}

	fn camera(&self) -> Camera {
		let c = self.camera.borrow();
		Camera {
			pos: c.pos,
			v10: c.target(),
			v68: (16840.0, 13640.0),
			i58: 0,
			b73: false,
			zoom: c.zoom,
		}
	}

	fn clock(&self) -> Clock {
		let (time, phase_seconds, phase) = self.clock.borrow().saved();
		Clock { time, phase_seconds, phase }
	}

	fn rosters(&self, vm: &SQVM) -> Result<Rosters, String> {
		self.units.original_rosters(vm)
	}

	fn fog(&self) -> Vec<u8> {
		self.grid.original_fog()
	}

	fn map_size(&self) -> (i64, i64) {
		self.grid.dims()
	}
}
