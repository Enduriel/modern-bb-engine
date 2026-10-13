//! Platform / build / misc engine stubs: build-flag predicates, achievements,
//! persistent stats, the `GameInfo` table, and assorted no-ops.
//!
//! Behavior comes from the host-supplied [`NativeConfig`](super::NativeConfig):
//! the build-flag predicates and `hasDLC` (which gate content loading) reflect its
//! fields, while achievements are no-ops and persistent stats are an in-memory
//! map. Standard base-lib functions (`collectgarbage`/`dummy`/`enabledebuginfo`/
//! `resurrectunreachable`) already come from the VM and are not redefined here.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use squirrel::{SQVM, SqRef, SqStrongRef, SqTable, Variadic};

use super::NativeConfig;

/// Register the platform/misc natives and the `GameInfo` table into the root.
pub(crate) fn install(vm: &mut SQVM, config: &NativeConfig) {
	// In-memory persistent stats (vanilla only uses the key 'CrisesDefeatedOnIronman').
	let stats: Rc<RefCell<HashMap<String, i64>>> = Rc::new(RefCell::new(HashMap::new()));

	// Copy/clone the config values the 'static closures need.
	let is_steam = config.is_steam_build;
	let is_devmode = config.is_devmode;
	let is_release = config.is_release_build;
	let is_demo = config.is_scenario_demo;
	let owned_dlc = config.owned_dlc.clone();
	let version_number = config.version_number.clone();
	let version_name = config.version_name.clone();
	let company = config.company.clone();

	// All `make_closure` calls precede the table handles (mutable vs shared borrow).
	let set_stat = {
		let stats = stats.clone();
		vm.make_closure(move |key: String, value: i64| {
			stats.borrow_mut().insert(key, value);
		})
	};
	let get_stat = {
		let stats = stats.clone();
		vm.make_closure(move |key: &str| -> i64 { stats.borrow().get(key).copied().unwrap_or(0) })
	};

	let root_fns: Vec<(&str, SqStrongRef)> = vec![
		("isSteamBuild", vm.make_closure(move || is_steam)),
		("isDevmode", vm.make_closure(move || is_devmode)),
		("isReleaseBuild", vm.make_closure(move || is_release)),
		("isScenarioDemo", vm.make_closure(move || is_demo)),
		(
			"hasDLC",
			vm.make_closure(move |app_id: i64| owned_dlc.contains(&app_id)),
		),
		("hasAchievement", vm.make_closure(|_id: &str| false)),
		(
			"updateAchievement",
			vm.make_closure(|_id: &str, _value: i64, _max: i64| {}),
		),
		("storeStatistics", vm.make_closure(|| {})),
		("__ping", vm.make_closure(|| {})),
		("isModified", vm.make_closure(|| false)),
		// A root global the original answers `false` for (probed 2026-10-13); scripts use
		// it to gate RNG-consuming branches, so the value matters for seed parity.
		("isSomethingToSee", vm.make_closure(|| false)),
		// `crash` is a debug tool that takes any argument(s); a no-op here.
		("crash", vm.make_closure(|_args: Variadic<SqRef>| {})),
		("reloadTextures", vm.make_closure(|| {})),
		("setPersistentStat", set_stat),
		("getPersistentStat", get_stat),
	];

	let game_info_fns: Vec<(&str, SqStrongRef)> = vec![
		(
			"getVersionNumber",
			vm.make_closure(move || version_number.clone()),
		),
		(
			"getVersionName",
			vm.make_closure(move || version_name.clone()),
		),
		("getBuildName", vm.make_closure(|| "modern".to_string())),
		("getRevisionNumber", vm.make_closure(|| 0i64)),
		("getBuildDate", vm.make_closure(|| String::new())),
		("getCompany", vm.make_closure(move || company.clone())),
	];

	let mut root = SqTable::root(vm);
	for (name, func) in root_fns {
		root.set(name, func);
	}

	let mut game_info = SqTable::new(vm);
	for (name, func) in game_info_fns {
		game_info.set(name, func);
	}
	root.set("GameInfo", game_info);
}
