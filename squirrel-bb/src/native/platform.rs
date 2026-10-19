//! Platform / build / misc engine stubs: build-flag predicates, achievements,
//! persistent stats, the `GameInfo` table, and assorted no-ops.
//!
//! Behavior comes from the host-supplied [`NativeConfig`](super::NativeConfig):
//! the build-flag predicates and `hasDLC` (which gate content loading) reflect its
//! fields. Achievements and persistent stats go to the host's [`UserStats`] service
//! (see [`super::user_stats`]). Standard base-lib functions (`collectgarbage`/`dummy`/`enabledebuginfo`/
//! `resurrectunreachable`) already come from the VM and are not redefined here.

use squirrel::{SQVM, SqRef, SqStrongRef, SqTable, Variadic};

use super::NativeConfig;
use super::user_stats::UserStatsHandle;

/// Register the platform/misc natives and the `GameInfo` table into the root. The
/// achievement and stat natives call `user_stats`' service.
pub(crate) fn install(vm: &mut SQVM, config: &NativeConfig, user_stats: &UserStatsHandle) {

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
	let mut root_fns: Vec<(&str, SqStrongRef)> = super::user_stats::install(vm, user_stats);
	root_fns.extend([
		("isSteamBuild", vm.make_closure(move || is_steam)),
		("isDevmode", vm.make_closure(move || is_devmode)),
		("isReleaseBuild", vm.make_closure(move || is_release)),
		("isScenarioDemo", vm.make_closure(move || is_demo)),
		(
			"hasDLC",
			vm.make_closure(move |app_id: i64| owned_dlc.contains(&app_id)),
		),
		("__ping", vm.make_closure(|| {})),
		("isModified", vm.make_closure(|| {
			super::stubs::note("isModified");
			false
		})),
		// A root global the original answers `false` for (probed 2026-10-13); scripts use
		// it to gate RNG-consuming branches, so the value matters for seed parity.
		("isSomethingToSee", vm.make_closure(|| false)),
		// `crash` is a debug tool that takes any argument(s); a no-op here.
		("crash", vm.make_closure(|_args: Variadic<SqRef>| {})),
		("reloadTextures", super::stubs::closure(vm, "reloadTextures")),
	]);

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
