//! The `World` engine object. For now only `World.getTime()`, which is the one
//! boot-critical piece: config scripts read `World.getTime().SecondsPerDay` (and
//! `SecondsPerHour`) at table-construction time, so it must exist before the
//! preload sweep. The rest of the `World` API (spawnEntity, getTile, …) belongs to
//! the World/Campaign subsystem and lands later.

use std::cell::Cell;
use std::rc::Rc;

use squirrel::{Retained, SQVM, SqStrongRef, SqTable, Variadic};

use super::NativeConfig;

/// Register the `World` table (`getTime`, `getPlayerRoster`) into the root.
pub(crate) fn install(vm: &mut SQVM, config: &NativeConfig) {
	let seconds_per_day = config.seconds_per_day;
	let seconds_per_hour = config.seconds_per_hour;

	// `getTime()` returns a fresh snapshot of the world clock. The constants are
	// read at config-load time; the live fields (Days/Hours/TimeOfDay/IsDaytime)
	// are zero/default until a running World clock supplies them.
	let get_time = vm.make_closure(move |vm: &SQVM| -> SqStrongRef {
		let mut time = SqTable::new(vm);
		time.set("SecondsPerDay", seconds_per_day);
		time.set("SecondsPerHour", seconds_per_hour);
		time.set("Days", 0i64);
		time.set("Hours", 0i64);
		time.set("TimeOfDay", 0i64);
		time.set("Time", 0i64);
		time.set("IsDaytime", true);
		time.to_strong()
	});

	// Rosters (getRoster/getPlayerRoster/createRoster/deleteRoster) are provided by the
	// host-owned `TacticalEntities` service (real rosters whose `create` mints units),
	// installed after this — see native/tactical_entities.rs.

	// Save/load hook registration (world_state.onInit registers these). Save-compat is
	// not a goal, so they just accept-and-ignore the callback.
	let set_on_load = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let set_on_before_load = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let set_on_save = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let set_on_before_save = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});

	// World speed multiplier (0 = paused, 1 = normal, higher = fast-forward). A shared
	// scalar the world clock will eventually drive; for now setPause/setNormalTime just
	// store + read it. (Real virtual-time scaling integrates with TimeService later.)
	let speed_mult = Rc::new(Cell::new(1.0_f64));
	let get_speed_mult = {
		let s = speed_mult.clone();
		vm.make_closure(move || s.get())
	};
	let set_speed_mult = {
		let s = speed_mult.clone();
		vm.make_closure(move |m: f64| s.set(m))
	};

	let mut world = SqTable::new(vm);
	world.set("getTime", get_time);
	world.set("setOnLoadCallback", set_on_load);
	world.set("setOnBeforeLoadCallback", set_on_before_load);
	world.set("setOnSaveCallback", set_on_save);
	world.set("setOnBeforeSaveCallback", set_on_before_save);
	world.set("getSpeedMult", get_speed_mult);
	world.set("setSpeedMult", set_speed_mult);
	SqTable::root(vm).set("World", world);
}
