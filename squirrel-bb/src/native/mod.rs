//! Battle Brothers native global functions and value classes.
//!
//! These are engine-provided globals scripts call but that have no `.nut`
//! definition. They are built on the generic native-class / native-function API
//! from the `squirrel` crate (per the crate boundary: the mechanism is generic
//! and lives in `squirrel`; these concrete BB types live here).
//!
//! Covers the value classes ([`vec`]: `vec2f`/`vec2i` + `createVec`/
//! `getVecDistance`/`getVecDir`; [`color`]: `color` + `createColor`/etc), the
//! [`math`] table (incl. the [`prng`]-backed `rand`/`seedRandom`), [`strings`]
//! (the `String` table + `buildTextFromTemplate`/`isFirstCharacter`/…), [`log`]
//! (logging → a host handler), and [`platform`] (build flags / achievements /
//! `GameInfo` stubs). The engine-state natives (Time scheduler, World) land later.

mod audio;
mod brushes;
mod color;
mod display_modes;
mod identity;
mod inject;
mod log;
mod math;
mod overload;
mod platform;
mod mt64;
mod prng;
mod settings;
mod strings;
mod tactical;
pub(crate) mod tactical_grid;
mod tactical_entities;
mod tactical_nav;
pub(crate) mod hex_range;
mod weather;
mod time;
mod ui_bridge;
mod vec;
mod world;
mod world_entities;
pub(crate) mod world_detail;
mod world_grid;
mod world_layout;
mod world_query;
pub(crate) mod world_time;
pub(crate) mod save;
mod world_nav;
mod world_search;

pub use brushes::Sprites;
pub use color::Color;
pub use log::{LogLevel, install_logging};
pub use tactical_entities::TacticalEntities;
pub use tactical_grid::TacticalGrid;
pub use tactical_nav::TacticalNavigator;
pub use time::TimeService;
pub use ui_bridge::{UiBridge, UiCall, UiFrontend};
pub use vec::{Vec2f, Vec2i};
pub use world_entities::WorldEntities;
pub use save::SaveSystem;
pub use world_grid::WorldGrid;

use squirrel::SQVM;

/// Host-supplied configuration for the build-dependent natives.
///
/// These affect *which content loads* (Steam/DLC/dev gates) and world-time
/// scaling, so the host sets them rather than relying on a fixed default. Use
/// [`NativeConfig::default`] then override the fields you care about.
#[derive(Clone, Debug)]
pub struct NativeConfig {
	/// `isSteamBuild()`.
	pub is_steam_build: bool,
	/// `isDevmode()` — gates developer tools.
	pub is_devmode: bool,
	/// `isReleaseBuild()`.
	pub is_release_build: bool,
	/// `isScenarioDemo()`.
	pub is_scenario_demo: bool,
	/// Owned DLC app IDs; `hasDLC(appId)` is true iff `appId` is in this list.
	/// Default empty (base game) — the host adds the IDs the player owns.
	pub owned_dlc: Vec<i64>,
	/// `GameInfo.getVersionNumber()`.
	pub version_number: String,
	/// `GameInfo.getVersionName()`.
	pub version_name: String,
	/// `GameInfo.getCompany()`.
	pub company: String,
}

/// Each paid DLC's init script and its Steam app ID (the argument the script passes
/// to `hasDLC`). Every DLC ships its init script in its own `.dat` archive, which
/// Steam installs only for owners, so "the script is installed" is the ownership
/// signal; see [`installed_dlc`].
pub const DLC_SCRIPTS: [(&str, i64); 6] = [
	// Fangshire grants the legendary helmet with every new campaign (it pushes it onto
	// `Const.World.Assets.NewCampaignEquipment`). Its id is read from the script's
	// bytecode (the decompiled scripts lack it).
	("scripts/dlc/dlc_fangshire", 365_382),
	("scripts/dlc/dlc_lindwurm", 732_460),
	("scripts/dlc/dlc_unhold", 961_930),
	("scripts/dlc/dlc_wildmen", 1_067_690),
	("scripts/dlc/dlc_desert", 1_361_280),
	("scripts/dlc/dlc_paladins", 1_910_050),
];

/// The app IDs of the DLCs whose init scripts `source` provides, for
/// [`NativeConfig::owned_dlc`]. The original additionally re-checks ownership with
/// Steam inside `hasDLC`; we rely on Steam having installed only owned DLC archives.
pub fn installed_dlc<S: crate::ScriptSource>(source: &S) -> Vec<i64> {
	let present = source.list_prefix("scripts/dlc/").unwrap_or_default();
	DLC_SCRIPTS
		.iter()
		.filter(|(script, _)| present.iter().any(|p| p == script))
		.map(|&(_, id)| id)
		.collect()
}

impl Default for NativeConfig {
	fn default() -> Self {
		NativeConfig {
			is_steam_build: false,
			is_devmode: false,
			is_release_build: true,
			is_scenario_demo: false,
			owned_dlc: Vec::new(),
			// The values the original reports for the 1.5.2.3 data these scripts come
			// from (probed 2026-10-13); mods read them (Modern Hooks registers
			// "vanilla" with this version).
			version_number: "1.5.2.3".to_string(),
			version_name: String::new(),
			company: "(c) Overhype Studios GmbH".to_string(),
		}
	}
}

impl NativeConfig {
	/// Install all native globals into the VM's root table, using this config for
	/// the build-dependent ones.
	///
	/// Call once, before running game scripts (several natives — `createVec`,
	/// `createColor`, `World.getTime`, `isSteamBuild`, `hasDLC` — are invoked at
	/// config-load time, so they must exist before the preload sweep). Logging gets
	/// the default stderr/stdout handler; call [`install_logging`] afterward to
	/// route logs elsewhere.
	///
	/// This installs the *stateless* natives only. The two host-managed, stateful
	/// services are installed separately (the host holds them to drive their
	/// state): the module system ([`ModuleSystem`](crate::ModuleSystem)) and the
	/// clocks ([`TimeService`] — also needed before preload, since config scripts
	/// read `Time.*`).
	pub fn install(&self, vm: &mut SQVM) -> InstalledNatives {
		// One shared RNG stream feeds Math.rand and color.varyRGB.
		let prng = prng::Prng::new();
		vec::install(vm);
		color::install(vm, prng.clone());
		math::install(vm, prng);
		strings::install(vm);
		platform::install(vm, self);
		let world_clock = world::install(vm);
		tactical::install(vm);
		let sprites = brushes::install(vm);
		settings::install(vm);
		audio::install(vm);
		identity::install(vm);
		install_logging(vm, log::default_handler);
		InstalledNatives { world_clock, sprites }
	}
}

/// Handles to state the stateless-looking natives keep, for the host (e.g. to save it).
pub struct InstalledNatives {
	pub(crate) world_clock: world_time::Clock,
	pub(crate) sprites: Sprites,
}

/// Install all native globals with the [default](NativeConfig::default) config.
/// Convenience for [`NativeConfig::default().install(vm)`](NativeConfig::install).
pub fn install_natives(vm: &mut SQVM) {
	NativeConfig::default().install(vm);
}

#[cfg(test)]
mod dlc_tests {
	use super::installed_dlc;
	use crate::{LoadedScript, ScriptSource};

	struct Listing(Vec<String>);

	impl ScriptSource for Listing {
		fn load(&self, _path: &str) -> Option<LoadedScript> {
			None
		}
		fn list_prefix(&self, prefix: &str) -> Option<Vec<String>> {
			let hits: Vec<String> = self.0.iter().filter(|p| p.starts_with(prefix)).cloned().collect();
			(!hits.is_empty()).then_some(hits)
		}
	}

	#[test]
	fn ownership_follows_installed_dlc_scripts() {
		let source = Listing(vec![
			"scripts/dlc/dlc_fangshire".to_string(),
			"scripts/dlc/dlc_desert".to_string(),
			"scripts/dlc/dlc_unhold".to_string(),
		]);
		assert_eq!(installed_dlc(&source), vec![365_382, 961_930, 1_361_280]);
		assert!(installed_dlc(&Listing(vec![])).is_empty()); // no scripts/dlc at all
	}
}
