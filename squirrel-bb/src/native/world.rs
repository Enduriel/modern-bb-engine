//! The `World` engine table and its stateless or small pieces: the clock (see
//! [`super::world_time`]; config scripts read `World.getTime().SecondsPerDay` at load
//! time, so it must exist before them), save hooks, camera and weather. The grid,
//! entities and rosters are added by their own services.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqStrongRef, SqTable};

use super::world_clouds::{CloudCover, CloudSettings, Clouds, MapSpan};
use super::world_layout::tile_pos;
use super::world_waves::Waves;

/// A script's `createCloudSettings()` table as [`CloudSettings`]; fields it doesn't
/// hold keep the original's defaults.
pub(super) fn cloud_settings(vm: &SQVM, table: &SqStrongRef) -> CloudSettings {
	let d = CloudSettings::default();
	let f = |key: &str, default: f32| table.get_member::<Number>(vm, key).map_or(default, |n| n.0 as f32);
	let i = |key: &str, default: i64| table.get_member::<Number>(vm, key).map_or(default, |n| n.0 as i64);
	let color = table
		.get_member::<SqStrongRef>(vm, "Color")
		.and_then(|c| super::color::color_argb(vm, &c))
		.unwrap_or(d.color);
	CloudSettings {
		min_clouds: i("MinClouds", d.min_clouds),
		max_clouds: i("MaxClouds", d.max_clouds),
		min_velocity: f("MinVelocity", d.min_velocity),
		max_velocity: f("MaxVelocity", d.max_velocity),
		min_alpha: f("MinAlpha", d.min_alpha),
		max_alpha: f("MaxAlpha", d.max_alpha),
		min_scale: f("MinScale", d.min_scale),
		max_scale: f("MaxScale", d.max_scale),
		kind: i("Type", d.kind),
		color,
	}
}

/// The centres of the map's first and last tiles (`World.getMapSize()`), or `None`
/// without a map.
fn map_span(vm: &SQVM) -> Result<Option<MapSpan>, String> {
	let world = SqTable::root(vm).get::<SqStrongRef>("World").ok_or("World missing")?;
	let size = world.call_method_ret::<SqStrongRef, _>(vm, "getMapSize", ())?;
	let dim = |axis: &str| size.as_ref().and_then(|s| s.get_member::<Number>(vm, axis)).map_or(0, |n| n.0 as i64);
	let (w, h) = (dim("X"), dim("Y"));
	if w <= 0 || h <= 0 {
		return Ok(None);
	}
	let (min, max) = (tile_pos(0, 0), tile_pos(w - 1, h - 1));
	Ok(Some(MapSpan {
		min: (min.0 as f32, min.1 as f32),
		max: (max.0 as f32, max.1 as f32),
	}))
}


/// Register the `World` table (clock, save hooks, camera, weather) into the root, and
/// return the world clock.
pub(crate) fn install(
	vm: &mut SQVM,
	engine_rng: &super::EngineRng,
	bird_rng: &super::EngineRng,
	sprites: &super::Sprites,
) -> (
	super::world_time::Clock,
	Clouds,
	super::world_birds::Birds,
	Waves,
	super::world_camera::WorldCamera,
	super::world_camera::WorldCameraCallbacks,
	super::particles::ParticlesHandle,
) {
	let world_clock: super::world_time::Clock = Default::default();
	let clouds: Clouds = Rc::new(RefCell::new(CloudCover::new(engine_rng.clone())));
	let birds: super::world_birds::Birds =
		Rc::new(RefCell::new(super::world_birds::Flock::new(bird_rng.clone(), sprites.clone())));
	let waves: Waves = Rc::default();
	// Rosters (getRoster/getPlayerRoster/createRoster/deleteRoster) are provided by the
	// host-owned `TacticalEntities` service (real rosters whose `create` mints units),
	// installed after this — see native/tactical_entities.rs.

	// Save/load hook registration (world_state.onInit registers these). Save-compat is
	// not a goal, so they just accept-and-ignore the callback.
	let set_on_load = super::stubs::closure(vm, "World.setOnLoadCallback");
	let set_on_before_load = super::stubs::closure(vm, "World.setOnBeforeLoadCallback");
	let set_on_save = super::stubs::closure(vm, "World.setOnSaveCallback");
	let set_on_before_save = super::stubs::closure(vm, "World.setOnBeforeSaveCallback");

	// World.getCamera(): see `world_camera`. World.screenToTile(x, y): the tile under a
	// pixel.
	let (camera, world_camera, camera_callbacks) = super::world_camera::instance(vm);
	let get_camera = vm.make_closure(move || camera.clone());
	let screen_to_tile = {
		let camera = world_camera.clone();
		vm.make_closure(move |vm: &SQVM, x: Number, y: Number| {
			super::world_camera::screen_to_tile(vm, &camera, x.0 as i32, y.0 as i32)
		})
	};

	// World.getWeather(): visual only (see `weather`), but its clouds are kept for the
	// renderer (see `world_clouds`).
	let weather = super::weather::weather(vm, "World");
	let build_cloud_cover = {
		let clouds = clouds.clone();
		vm.make_closure(move |vm: &SQVM, settings: Retained| -> Result<(), String> {
			let settings = cloud_settings(vm, &settings.0);
			match map_span(vm)? {
				Some(span) => clouds.borrow_mut().build(settings, span),
				None => clouds.borrow_mut().clear(),
			}
			Ok(())
		})
	};
	weather.set_member(vm, "buildCloudCover", build_cloud_cover).expect("the weather table");
	// buildBirds(settings): see `world_birds`.
	let build_birds = {
		let birds = birds.clone();
		vm.make_closure(move |vm: &SQVM, settings: Retained| -> Result<(), String> {
			let num = |name: &str| settings.0.get_member::<Number>(vm, name).map_or(0.0, |n| n.0);
			let s = super::world_birds::BirdSettings {
				min_birds: num("MinBirds") as i64,
				max_birds: num("MaxBirds") as i64,
				min_velocity: num("MinVelocity") as f32,
				max_velocity: num("MaxVelocity") as f32,
			};
			match map_span(vm)? {
				Some(span) => birds.borrow_mut().build(s, span),
				None => birds.borrow_mut().clear(),
			}
			Ok(())
		})
	};
	weather.set_member(vm, "buildBirds", build_birds).expect("the weather table");
	let get_weather = vm.make_closure(move || weather.clone());

	// Footprints are the world entities' (see `world_entities::footprints`); particle
	// effects are `particles`', set on the table below.
	let particles: super::particles::ParticlesHandle =
		Rc::new(RefCell::new(super::particles::Particles::new(engine_rng.clone())));
	let on_update: Rc<dyn Fn(&SQVM, f32, f32)> = {
		let (clouds, birds, waves, particles) = (clouds.clone(), birds.clone(), waves.clone(), particles.clone());
		Rc::new(move |vm: &SQVM, delta, speed| {
			clouds.borrow_mut().advance(delta, speed);
			birds.borrow_mut().advance(delta, speed);
			birds.borrow_mut().flap(super::sprite_anim::virtual_ms(vm));
			waves.borrow_mut().advance(delta, speed);
			super::particles::advance_now(vm, &particles, delta);
		})
	};
	let clock = super::world_time::world_entries(vm, &world_clock, on_update);
	// World.spawnWaveSprite(brush, pos, drift, lifetime): see `world_waves`. An unknown
	// brush spawns nothing.
	let spawn_wave_sprite = {
		let (waves, sprites) = (waves.clone(), sprites.clone());
		vm.make_closure(move |vm: &SQVM, brush: &str, pos: Retained, drift: Retained, lifetime: Number| -> Result<(), String> {
			let xy = |v: &SqStrongRef| super::vec::position_f32(vm, v, "spawnWaveSprite");
			let (pos, drift) = (xy(&pos.0)?, xy(&drift.0)?);
			if sprites.with(|r| r.contains(brush)) {
				waves.borrow_mut().spawn(brush, pos, drift, lifetime.0 as f32);
			}
			Ok(())
		})
	};
	// World.setVisible/isVisible: whether the campaign map is drawn (the world state
	// hides it during a battle).
	let visible = std::rc::Rc::new(std::cell::Cell::new(true));
	let set_visible = {
		let v = visible.clone();
		vm.make_closure(move |on: bool| v.set(on))
	};
	let is_visible = vm.make_closure(move || visible.get());
	// World.move(from, to, max): `from` moved toward `to` by at most `max` pixels.
	let step = vm.make_closure(|vm: &SQVM, from: Retained, to: Retained, max: Number| -> Result<Option<SqStrongRef>, String> {
		let xy = |v: &SqStrongRef| super::vec::position(vm, v, "World.move");
		let (x, y) = super::world_layout::step_toward(xy(&from.0)?, xy(&to.0)?, max.0);
		Ok(SqTable::root(vm)
			.get::<SqStrongRef>("createVec")
			.and_then(|create| create.call_ret::<SqStrongRef, _>(vm, (x as f64, y as f64)).ok().flatten()))
	});
	let mut world = SqTable::new(vm);
	world.set("move", step);
	world.set("getCamera", get_camera);
	world.set("screenToTile", screen_to_tile);
	world.set("setVisible", set_visible);
	world.set("isVisible", is_visible);
	world.set("getWeather", get_weather);
	world.set("spawnWaveSprite", spawn_wave_sprite);
	world.set("setOnLoadCallback", set_on_load);
	world.set("setOnBeforeLoadCallback", set_on_before_load);
	world.set("setOnSaveCallback", set_on_save);
	world.set("setOnBeforeSaveCallback", set_on_before_save);
	for (name, closure) in clock {
		world.set(name, closure);
	}
	SqTable::root(vm).set("World", world);
	super::particles::install_on(vm, "World", &particles);
	(world_clock, clouds, birds, waves, world_camera, camera_callbacks, particles)
}
