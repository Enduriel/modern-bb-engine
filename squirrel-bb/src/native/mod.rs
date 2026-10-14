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

pub(crate) mod audio;
pub(crate) mod battle_effects;
mod brushes;
pub(crate) mod camera_motion;
mod color;
mod engine_rng;
mod entity_ids;
mod execute;
pub use entity_ids::EntityIds;
pub(crate) mod fog_mask;
mod display_modes;
mod identity;
mod inject;
mod log;
mod math;
pub(crate) mod original_world;
mod overload;
pub(crate) mod particles;
mod platform;
mod mt64;
pub(crate) mod prng;
mod screen;
mod settings;
pub(crate) mod spin;
pub(crate) use settings::temp_gameplay_flag;
pub(crate) mod sprite_color;
pub(crate) mod sprite_slots;
mod strings;
mod text_template;
pub(crate) mod tactical;
pub(crate) mod tactical_grid;
mod tactical_entities;
mod tactical_nav;
pub(crate) mod hex_range;
mod tile_of_type;
mod weather;
mod time;
mod ui_bridge;
pub(crate) mod ui_cursor;
mod vec;
mod world;
pub(crate) mod world_camera;
mod world_entities;
pub(crate) mod world_detail;
mod world_grid;
pub(crate) mod world_clouds;
pub(crate) mod world_waves;
pub(crate) mod world_clear_scene;
mod world_layout;
mod world_query;
pub(crate) mod world_time;
pub(crate) mod save;
mod world_nav;
mod world_search;

pub use brushes::Sprites;
pub use color::Color;
pub use engine_rng::EngineRng;
pub use settings::VideoSettings;
pub use log::{LogLevel, install_logging};
pub use tactical_entities::TacticalEntities;
pub use tactical_grid::TacticalGrid;
pub use tactical_nav::{BattleMotion, TacticalNavigator};
pub use time::{ExactClock, TimeService};
pub use tactical_grid::camera::BattleCamera;
pub use ui_bridge::{UiBridge, UiCall, UiFrontend};
pub use vec::{Vec2f, Vec2i};
pub use world_entities::WorldEntities;
pub use save::original::compare::{compare_original_saves, original_save_name};
pub use save::{SaveFormat, SaveSystem};
pub use world_grid::WorldGrid;
pub use fog_mask::{FogPatch, FogStamp, MASK_SIZE as FOG_MASK_SIZE};

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
	/// Seed of the engine's own random stream ([`EngineRng`]): `None` seeds it
	/// unpredictably, as the original does; a fixed seed makes runs reproducible.
	pub engine_seed: Option<u64>,
	/// The clock behind `Time.getExactTime()`: frame time (the default, deterministic)
	/// or live, as the original's (the windowed game).
	pub exact_clock: ExactClock,
	/// How the battle camera moves: at once (the default, deterministic) or gliding,
	/// as the original's (the windowed game).
	pub battle_camera: BattleCamera,
	/// How units move in a battle: a phase of their hop a call (the default,
	/// deterministic) or over time, as the original's (the windowed game).
	pub battle_motion: BattleMotion,
}

/// Mixed into `NativeConfig::engine_seed` for the texts' own stream
/// (`buildTextFromTemplate`'s choices), so a seeded run is reproducible.
const TEXT_SEED: u64 = 0x7E47_5EED_0000_0001;

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
			engine_seed: None,
			exact_clock: ExactClock::Frame,
			battle_camera: BattleCamera::Instant,
			battle_motion: BattleMotion::Stepped,
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
		// Pools draw from the C runtime's rand(), which 0-argument Math.rand also is.
		let crt = prng.clone();
		vec::install(vm);
		color::install(vm, prng.clone());
		math::install(vm, prng);
		// The texts' random choices have a stream of their own, as the original's.
		strings::install(vm, self.engine_seed.map_or_else(EngineRng::unseeded, |s| EngineRng::seeded(s ^ TEXT_SEED)));
		platform::install(vm, self);
		let engine_rng = self.engine_seed.map_or_else(EngineRng::unseeded, EngineRng::seeded);
		let sprites = brushes::install(vm);
		let (world_clock, clouds, waves, world_camera, world_particles) = world::install(vm, &engine_rng, &sprites);
		let tactical = tactical::install(vm, &engine_rng, &crt);
		let video = settings::install(vm);
		execute::install(vm);
		identity::install(vm);
		install_logging(vm, log::default_handler);
		InstalledNatives { world_clock, clouds, waves, world_camera, world_particles, tactical, sprites, engine_rng, video }
	}
}

/// Handles to state the stateless-looking natives keep, for the host (e.g. to save it).
pub struct InstalledNatives {
	pub(crate) world_clock: world_time::Clock,
	pub(crate) clouds: world_clouds::Clouds,
	pub(crate) waves: world_waves::Waves,
	pub(crate) world_camera: world_camera::WorldCamera,
	pub(crate) world_particles: particles::ParticlesHandle,
	pub(crate) tactical: tactical::TacticalParts,
	pub(crate) sprites: Sprites,
	pub(crate) engine_rng: EngineRng,
	pub(crate) video: VideoSettings,
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
