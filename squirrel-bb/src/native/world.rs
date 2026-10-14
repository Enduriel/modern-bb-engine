//! The `World` engine table and its stateless or small pieces: the clock (see
//! [`super::world_time`]; config scripts read `World.getTime().SecondsPerDay` at load
//! time, so it must exist before them), save hooks, camera and weather. The grid,
//! entities and rosters are added by their own services.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqStrongRef, SqTable, Variadic};

use super::world_clouds::{CloudCover, CloudSettings, Clouds, MapSpan};
use super::world_layout::tile_pos;

/// A script's `createCloudSettings()` table as [`CloudSettings`]; fields it doesn't
/// hold keep the original's defaults.
fn cloud_settings(vm: &SQVM, table: &SqStrongRef) -> CloudSettings {
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
pub(crate) fn install(vm: &mut SQVM, engine_rng: &super::EngineRng) -> (super::world_time::Clock, Clouds) {
	let world_clock: super::world_time::Clock = Default::default();
	let clouds: Clouds = Rc::new(RefCell::new(CloudCover::new(engine_rng.clone())));
	// Rosters (getRoster/getPlayerRoster/createRoster/deleteRoster) are provided by the
	// host-owned `TacticalEntities` service (real rosters whose `create` mints units),
	// installed after this — see native/tactical_entities.rs.

	// Save/load hook registration (world_state.onInit registers these). Save-compat is
	// not a goal, so they just accept-and-ignore the callback.
	let set_on_load = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let set_on_before_load = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let set_on_save = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let set_on_before_save = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});

	// World.getCamera(): a headless camera. Scripts move/zoom it (no-ops here) and read
	// back `Zoom` (a plain, persistent table field) and `isInsideScreen`, which only
	// gates visual fades; with no screen it is always false.
	let camera = {
		let noop = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
		let inside = vm.make_closure(|Variadic(_a): Variadic<Retained>| false);
		let screen_to_world = vm.make_closure(|vm: &SQVM, Variadic(_a): Variadic<Retained>| -> Option<SqStrongRef> {
			SqTable::root(vm)
				.get::<SqStrongRef>("createVec")
				.and_then(|create| create.call_ret::<SqStrongRef, _>(vm, (0.0, 0.0)).ok().flatten())
		});
		let mut cam = SqTable::new(vm);
		cam.set("Zoom", 1.0);
		for name in ["setPos", "moveTo", "moveToPos", "zoomTo", "zoomBy", "move", "update", "jumpTo"] {
			cam.set(name, noop.clone());
		}
		cam.set("isInsideScreen", inside);
		cam.set("screenToWorld", screen_to_world);
		cam.to_strong()
	};
	let get_camera = {
		let cam = camera.clone();
		vm.make_closure(move || cam.clone())
	};

	// World.getWeather(): visual only (see `weather`), but its clouds are kept for the
	// renderer (see `world_clouds`).
	let weather = super::weather::weather(vm);
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
	let get_weather = vm.make_closure(move || weather.clone());

	// Footprints only feed the hover tooltip (and expire after 60 s); headless they are
	// not kept. Particle effects are visual, and no script uses the returned effect.
	let spawn_footprint = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let spawn_particle_effect = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let clock = super::world_time::world_entries(vm, &world_clock, &clouds);
	// World.setVisible/isVisible: whether the campaign map is drawn (the world state
	// hides it during a battle).
	let visible = std::rc::Rc::new(std::cell::Cell::new(true));
	let set_visible = {
		let v = visible.clone();
		vm.make_closure(move |on: bool| v.set(on))
	};
	let is_visible = vm.make_closure(move || visible.get());
	let mut world = SqTable::new(vm);
	world.set("getCamera", get_camera);
	world.set("setVisible", set_visible);
	world.set("isVisible", is_visible);
	world.set("getWeather", get_weather);
	world.set("spawnFootprint", spawn_footprint);
	world.set("spawnParticleEffect", spawn_particle_effect);
	world.set("setOnLoadCallback", set_on_load);
	world.set("setOnBeforeLoadCallback", set_on_before_load);
	world.set("setOnSaveCallback", set_on_save);
	world.set("setOnBeforeSaveCallback", set_on_before_save);
	for (name, closure) in clock {
		world.set(name, closure);
	}
	SqTable::root(vm).set("World", world);
	(world_clock, clouds)
}
