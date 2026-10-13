//! The `World` engine table and its stateless or small pieces: the clock (see
//! [`super::world_time`]; config scripts read `World.getTime().SecondsPerDay` at load
//! time, so it must exist before them), save hooks, camera and weather. The grid,
//! entities and rosters are added by their own services.

use squirrel::{Retained, SQVM, SqStrongRef, SqTable, Variadic};


/// Register the `World` table (clock, save hooks, camera, weather) into the root, and
/// return the world clock.
pub(crate) fn install(vm: &mut SQVM) -> super::world_time::Clock {
	let world_clock: super::world_time::Clock = Default::default();
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

	// World.getWeather(): clouds, birds and lighting are visual only, and building them
	// draws nothing from the script RNG (probed 2026-10-13), so headless they are no-ops.
	// The settings objects only receive field writes from scripts, so each is a table
	// pre-filled with every field vanilla writes (an absent key can't be assigned).
	let weather = {
		let noop = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
		const CLOUD: &[&str] = &[
			"Color", "Direction", "MaxAlpha", "MaxClouds", "MaxScale", "MaxVelocity", "MinAlpha",
			"MinClouds", "MinScale", "MinVelocity", "RandomizeDirection", "RandomizeRotation", "Sprite", "Type",
		];
		const BIRD: &[&str] = &["MaxBirds", "MaxVelocity", "MinBirds", "MinVelocity"];
		const RAIN: &[&str] = &[
			"MaxAlpha", "MaxDrops", "MaxScale", "MaxVelocity", "MinAlpha", "MinDrops", "MinScale",
			"MinVelocity", "NumSplats",
		];
		let creators: Vec<(&str, SqStrongRef)> = [
			("createCloudSettings", CLOUD),
			("createBirdSettings", BIRD),
			("createRainSettings", RAIN),
		]
		.into_iter()
		.map(|(name, fields)| {
			let create = vm.make_closure(move |vm: &SQVM| {
				let mut t = SqTable::new(vm);
				for f in fields {
					t.set(f, 0.0);
				}
				t.to_strong()
			});
			(name, create)
		})
		.collect();
		let mut w = SqTable::new(vm);
		w.set("IsRaining", false);
		for (name, create) in creators {
			w.set(name, create);
		}
		for name in ["buildCloudCover", "buildBirds", "buildRain", "setAmbientLightingPreset", "setAmbientLightingSaturation"] {
			w.set(name, noop.clone());
		}
		w.to_strong()
	};
	let get_weather = vm.make_closure(move || weather.clone());

	// Footprints only feed the hover tooltip (and expire after 60 s); headless they are
	// not kept. Particle effects are visual, and no script uses the returned effect.
	let spawn_footprint = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let spawn_particle_effect = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let clock = super::world_time::world_entries(vm, &world_clock);
	let mut world = SqTable::new(vm);
	world.set("getCamera", get_camera);
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
	world_clock
}
