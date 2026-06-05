//! The `World` engine object. For now only `World.getTime()`, which is the one
//! boot-critical piece: config scripts read `World.getTime().SecondsPerDay` (and
//! `SecondsPerHour`) at table-construction time, so it must exist before the
//! preload sweep. The rest of the `World` API (spawnEntity, getTile, …) belongs to
//! the World/Campaign subsystem and lands later.

use squirrel::{Retained, SQVM, SqArray, SqStrongRef, SqTable, Variadic};

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

	// A stub player roster — empty, with no-op clear/add. `main_menu_state.onShow`
	// calls `World.getPlayerRoster().clear()`. The real entity-backed roster (and
	// the rest of World) is the World subsystem; this is one shared empty
	// roster returned by `getPlayerRoster`.
	let roster = {
		let clear = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
		let add = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
		let get_all = vm.make_closure(|vm: &SQVM| SqArray::new(vm).to_strong());
		let get_size = vm.make_closure(|| 0i64);
		let mut r = SqTable::new(vm);
		r.set("clear", clear);
		r.set("add", add);
		r.set("getAll", get_all);
		r.set("getSize", get_size);
		r.to_strong()
	};
	let get_player_roster = {
		let roster = roster.clone();
		vm.make_closure(move || roster.clone())
	};
	// Faction/settlement rosters. Real entity-backed rosters are gameplay;
	// for worldgen, `settlement.onInit` calls `World.createRoster(getID())` and
	// `faction.getRoster()` reads `World.getRoster(id)`. Hand back the shared empty
	// roster stub for any id; create/delete are no-ops.
	let create_roster = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let delete_roster = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let get_roster = {
		let roster = roster.clone();
		vm.make_closure(move |Variadic(_a): Variadic<Retained>| roster.clone())
	};

	let mut world = SqTable::new(vm);
	world.set("getTime", get_time);
	world.set("getPlayerRoster", get_player_roster);
	world.set("createRoster", create_roster);
	world.set("deleteRoster", delete_roster);
	world.set("getRoster", get_roster);
	SqTable::root(vm).set("World", world);
}
