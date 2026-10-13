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

	// World.getWeather(): visual only (see `weather`).
	let weather = super::weather::weather(vm);
	let get_weather = vm.make_closure(move || weather.clone());

	// Footprints only feed the hover tooltip (and expire after 60 s); headless they are
	// not kept. Particle effects are visual, and no script uses the returned effect.
	let spawn_footprint = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let spawn_particle_effect = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let clock = super::world_time::world_entries(vm, &world_clock);
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
	world_clock
}
