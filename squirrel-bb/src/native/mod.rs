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
mod color;
mod identity;
mod log;
mod math;
mod overload;
mod platform;
mod prng;
mod settings;
mod strings;
mod tactical;
mod time;
mod ui_bridge;
mod vec;
mod world;
mod world_entities;
mod world_grid;

pub use color::Color;
pub use log::{LogLevel, install_logging};
pub use time::TimeService;
pub use ui_bridge::{UiBridge, UiCall};
pub use vec::{Vec2f, Vec2i};
pub use world_entities::WorldEntities;
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
	/// `World.getTime().SecondsPerDay` (the real value is unconfirmed; this default
	/// only affects timing scale, not load correctness).
	pub seconds_per_day: i64,
	/// `World.getTime().SecondsPerHour`.
	pub seconds_per_hour: i64,
	/// `GameInfo.getVersionNumber()`.
	pub version_number: String,
	/// `GameInfo.getVersionName()`.
	pub version_name: String,
	/// `GameInfo.getCompany()`.
	pub company: String,
}

impl Default for NativeConfig {
	fn default() -> Self {
		NativeConfig {
			is_steam_build: false,
			is_devmode: false,
			is_release_build: true,
			is_scenario_demo: false,
			owned_dlc: Vec::new(),
			seconds_per_day: 86_400,
			seconds_per_hour: 3_600,
			version_number: "1.0.0".to_string(),
			version_name: "Modern Engine".to_string(),
			company: "Overhype Studios".to_string(),
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
	pub fn install(&self, vm: &mut SQVM) {
		// One shared RNG stream feeds Math.rand and color.varyRGB.
		let prng = prng::Prng::new();
		vec::install(vm);
		color::install(vm, prng.clone());
		math::install(vm, prng);
		strings::install(vm);
		platform::install(vm, self);
		world::install(vm, self);
		tactical::install(vm);
		settings::install(vm);
		audio::install(vm);
		identity::install(vm);
		install_logging(vm, log::default_handler);
	}
}

/// Install all native globals with the [default](NativeConfig::default) config.
/// Convenience for [`NativeConfig::default().install(vm)`](NativeConfig::install).
pub fn install_natives(vm: &mut SQVM) {
	NativeConfig::default().install(vm);
}
