//! [`Engine`]: the host object that owns the VM and every stateful service, and
//! tears them down in the correct order.
//!
//! The four host services ([`ModuleSystem`], [`TimeService`], [`UiBridge`],
//! [`RootState`]) each hold [`SqStrongRef`](squirrel::SqStrongRef)s into the VM that
//! MUST be released before `sq_close` — and merely dropping a service handle is not
//! enough, because VM-resident closures keep its state alive until `sq_close` (which
//! is then too late: releasing during teardown is a use-after-free). So each service
//! has a `clear()`, and forgetting to call all four before closing the VM is an easy,
//! crashing mistake.
//!
//! `Engine` removes that footgun: it owns all of them, and its [`Drop`] `clear()`s
//! every service *while the VM is still fully alive*, then lets the `vm` field (the
//! last-declared field) drop last for a clean `sq_close`.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef};

use crate::error::ScriptError;
use crate::module::ModuleSystem;
use crate::native::save::SaveSystem;
use crate::native::world_time::ClockPart;
use crate::native::{NativeConfig, Sprites, TacticalEntities, TacticalGrid, TacticalNavigator, TimeService, UiBridge, VideoSettings, WorldEntities, WorldGrid};
use crate::source::ScriptSource;
use crate::state_stack::RootState;

/// Mixed into `NativeConfig::engine_seed` for the audio's own stream, so a seeded run
/// is reproducible without the audio sharing the engine's stream.
const AUDIO_SEED: u64 = 0xA0D1_05EE_D000_0001;

/// The engine host: the VM plus all stateful services, with correct teardown.
///
/// Field order is load-bearing: Rust drops fields top-to-bottom, so `vm` is declared
/// **last** (dropped last). The [`Drop`] impl `clear()`s the services first (while
/// the VM lives), so no `SqStrongRef` is ever released into a closing VM.
pub struct Engine<S: ScriptSource> {
	saves: SaveSystem,
	root_state: RootState,
	world_grid: WorldGrid,
	world_entities: WorldEntities,
	tactical_entities: TacticalEntities,
	tactical_grid: TacticalGrid,
	tactical_navigator: TacticalNavigator,
	ui: UiBridge,
	/// The headless browser side answering the UI (see [`crate::ui_client`]).
	ui_client: RefCell<crate::ui_client::UiClient>,
	/// Every loaded sprite (no VM handles, so no teardown).
	sprites: Sprites,
	/// The video mode the scripts set (no VM handles).
	video: VideoSettings,
	/// The world's clock (no VM handles), for the day's lighting.
	world_clock: crate::native::world_time::Clock,
	/// The world's clouds and waves (no VM handles).
	clouds: crate::native::world_clouds::Clouds,
	waves: crate::native::world_waves::Waves,
	/// The tactical terrain transitions (no VM handles).
	transitions: crate::native::tactical::Transitions,
	tactical_light: crate::native::tactical::TacticalLight,
	tactical_clouds: crate::native::world_clouds::Clouds,
	world_camera: crate::native::world_camera::WorldCamera,
	world_particles: crate::native::particles::ParticlesHandle,
	tactical_particles: crate::native::particles::ParticlesHandle,
	battle_effects: crate::native::battle_effects::BattleEffectsHandle,
	/// The game's sound (no VM handles).
	audio: crate::native::audio::Audio,
	time: TimeService,
	modules: ModuleSystem<S>,
	vm: SQVM,
}

impl<S: ScriptSource + 'static> Engine<S> {
	/// Build an engine over `source` with the default native config.
	pub fn new(source: S) -> Self {
		Self::with_config(source, &NativeConfig::default())
	}

	/// Build an engine over `source`, installing the stateless natives from `config`
	/// and all four stateful services. Stack size is the BB-typical 64K.
	pub fn with_config(source: S, config: &NativeConfig) -> Self {
		let mut vm = SQVM::new(1 << 16);
		let natives = config.install(&mut vm);

		let modules = ModuleSystem::new(source);
		modules.install(&mut vm);
		let time = TimeService::new();
		time.set_exact_clock(config.exact_clock);
		time.install(&mut vm);
		let ui = UiBridge::new();
		ui.install(&mut vm);
		// After the `World` table exists (config.install), add the grid natives.
		let world_grid = WorldGrid::new();
		world_grid.set_sprites(natives.sprites.clone());
		world_grid.set_engine_rng(natives.engine_rng.clone());
		world_grid.set_camera(natives.world_camera.clone());
		world_grid.install(&mut vm);
		// After the grid natives (entity `getTile` delegates to `World.getTile`).
		let world_entities = WorldEntities::new(modules.instantiator());
		world_entities.set_sprites(natives.sprites.clone());
		world_entities.install(&mut vm);
		// Details take their ids from the counter every entity shares.
		world_grid.set_entity_ids(world_entities.ids());
		// After the World table exists: the roster/tactical-unit host (provides
		// World.getRoster/getPlayerRoster/createRoster, replacing the bare stubs).
		let tactical_entities = TacticalEntities::new(modules.instantiator());
		// One id space for the world's entities and the battle's units, as the original's.
		tactical_entities.share_ids(world_entities.ids());
		tactical_entities.set_sprites(natives.sprites.clone());
		tactical_entities.set_battle_effects(natives.tactical.effects.clone());
		tactical_entities.install(&mut vm);
		// The battle map, with units and objects placed on it by the entity host.
		let tactical_grid = TacticalGrid::new();
		tactical_grid.set_file_reader(modules.file_reader());
		tactical_grid.set_sprites(natives.sprites.clone());
		tactical_grid.set_engine_rng(natives.engine_rng.clone());
		tactical_grid.set_battle_camera(config.battle_camera);
		tactical_grid.install(&mut vm);
		tactical_entities.attach_map(&tactical_grid);
		let tactical_navigator = TacticalNavigator::new(&tactical_grid, tactical_entities.units());
		tactical_navigator.install(&mut vm);
		crate::native::tactical::install_effects(&mut vm, &natives.tactical);
		// Silent until the host gives it an output; its own random stream, as the original's.
		let audio = crate::native::audio::Audio::new(
			Box::new(crate::native::audio::NullOut::new(modules.file_reader())),
			config.engine_seed.map_or_else(crate::native::EngineRng::unseeded, |s| crate::native::EngineRng::seeded(s ^ AUDIO_SEED)),
		);
		let world_camera = natives.world_camera.clone();
		audio.install(&mut vm, move || crate::native::world_camera::view(&world_camera).pos, tactical_grid.camera_reader());
		let root_state = RootState::new(modules.instantiator());
		root_state.install(&mut vm);
		// The world scene's per-frame update (entity thinkers) runs from updateScene.
		root_state.set_scene_update(world_entities.scene_update_hook());
		// The world keeps the tile under the mouse.
		root_state.set_mouse_listener(world_entities.mouse_listener(natives.world_camera.clone(), world_grid.dims_reader()));
		// Saving and loading, over every stateful part (in load order).
		let saves = SaveSystem::new();
		saves.install(&mut vm);
		// The world scene's drawing: what the player discovers, and the picture a
		// presave asked for.
		let render_world = world_entities.scene_render_hook();
		let take_presave = saves.presave_hook();
		root_state.set_scene_render(move |vm| {
			render_world(vm)?;
			take_presave(vm)
		});
		saves.add_part(Rc::new(ClockPart(natives.world_clock.clone())));
		saves.add_part(time.save_part());
		saves.add_part(world_grid.save_part());
		// Entities before rosters: units' onDeserialize reads the settlements (e.g.
		// character backgrounds build their description from them).
		saves.add_part(world_entities.save_part());
		saves.add_part(tactical_entities.save_part());
		saves.set_original_world(Rc::new(crate::native::original_world::EngineWorld {
			grid: world_grid.clone(),
			entities: world_entities.clone(),
			units: tactical_entities.clone(),
			time: time.clone(),
			clock: natives.world_clock.clone(),
			camera: natives.world_camera.clone(),
		}));

		Engine {
			saves,
			root_state,
			world_grid,
			world_entities,
			tactical_entities,
			tactical_grid,
			tactical_navigator,
			ui,
			ui_client: RefCell::new(crate::ui_client::UiClient::new()),
			sprites: natives.sprites,
			video: natives.video,
			world_clock: natives.world_clock.clone(),
			clouds: natives.clouds.clone(),
			waves: natives.waves.clone(),
			transitions: natives.tactical.transitions.clone(),
			tactical_light: natives.tactical.light.clone(),
			tactical_clouds: natives.tactical.clouds.clone(),
			world_camera: natives.world_camera.clone(),
			world_particles: natives.world_particles.clone(),
			tactical_particles: natives.tactical.particles.clone(),
			battle_effects: natives.tactical.effects.clone(),
			audio,
			time,
			modules,
			vm,
		}
	}

	// ---- accessors ----

	/// The VM (for reads / `&self` ops like `run_script`).
	pub fn vm(&self) -> &SQVM {
		&self.vm
	}
	/// The VM mutably (for the rare `&mut` op, e.g. setting handlers before boot).
	pub fn vm_mut(&mut self) -> &mut SQVM {
		&mut self.vm
	}
	pub fn modules(&self) -> &ModuleSystem<S> {
		&self.modules
	}
	/// The save system (e.g. to keep saves in a directory).
	pub fn saves(&self) -> &SaveSystem {
		&self.saves
	}
	pub fn time(&self) -> &TimeService {
		&self.time
	}
	/// The game's sound: give it an output with [`Audio::set_out`](crate::Audio::set_out)
	/// (the default is silent).
	pub fn audio(&self) -> &crate::native::audio::Audio {
		&self.audio
	}
	/// The video mode, shared with `Settings.getVideoMode`/`setVideoMode`.
	pub fn video(&self) -> &VideoSettings {
		&self.video
	}
	/// The sprite registry behind `doesBrushExist`; the host fills it at startup.
	pub fn sprites(&self) -> &Sprites {
		&self.sprites
	}
	/// The world map as the scripts have set it up, for a renderer. The fog of war's
	/// mask comes separately, as patches ([`WorldGrid::take_fog_patch`]).
	pub fn world_scene(&self) -> crate::scene::WorldScene {
		crate::scene::WorldScene {
			tiles: self.world_grid.scene_tiles(),
			entities: self.world_entities.scene_entities(&self.vm),
			extent: self.world_grid.extent(),
			fog_of_war: self.world_grid.fog_of_war(),
			region_texts: self.world_grid.region_texts(),
			light: {
				let (color, saturation) = self.world_clock.borrow().ambient();
				crate::scene::SceneLight { color, saturation }
			},
			clouds: self.clouds.borrow().clouds().to_vec(),
			footprints: self.world_entities.scene_footprints(&self.vm),
			waves: self.waves.borrow().scene(),
			camera: crate::native::world_camera::view(&self.world_camera),
			particles: self.world_particles.borrow().scene(),
		}
	}
	/// What entity `id`'s portrait draws: a battle unit (placed on the map or not, e.g.
	/// a brother in the roster) or a world entity.
	pub fn portrait(&self, id: i64) -> Option<crate::scene::ScenePortrait> {
		self.tactical_entities
			.portrait(&self.vm, id)
			.or_else(|| self.world_entities.portrait(&self.vm, id))
	}

	/// The battle map as the scripts have set it up, for a renderer.
	pub fn tactical_scene(&self) -> crate::scene::tactical::TacticalScene {
		let (effects, pools) = self
			.battle_effects
			.borrow_mut()
			.scene(
				self.time.virtual_seconds() as f32,
				|tile| self.tactical_grid.tile_world_pos(tile),
				|sprite| self.tactical_entities.sprite_look(&self.vm, sprite),
			);
		crate::scene::tactical::TacticalScene {
			effects,
			pools,
			transitions: self.transitions.borrow().clone(),
			entities: self.tactical_entities.scene_entities(&self.vm),
			light: {
				let (color, saturation) = self.tactical_light.get();
				crate::scene::SceneLight { color, saturation }
			},
			clouds: self.tactical_clouds.borrow().clouds().to_vec(),
			hide_trees: crate::native::temp_gameplay_flag(&self.vm, "HideTrees"),
			particles: self.tactical_particles.borrow().scene(),
			..self.tactical_grid.scene(crate::native::sprite_color::real_time(&self.vm))
		}
	}
	/// Let the headless UI client answer what the scripts asked of the UI since the
	/// last pump (see [`crate::ui_client::UiClient::pump`]).
	pub fn pump_ui(&self) -> Result<Vec<crate::UiCall>, String> {
		self.ui_client.borrow_mut().pump(&self.ui, &self.vm)
	}

	pub fn ui(&self) -> &UiBridge {
		&self.ui
	}
	pub fn root_state(&self) -> &RootState {
		&self.root_state
	}
	pub fn world_grid(&self) -> &WorldGrid {
		&self.world_grid
	}
	pub fn world_entities(&self) -> &WorldEntities {
		&self.world_entities
	}
	pub fn tactical_entities(&self) -> &TacticalEntities {
		&self.tactical_entities
	}

	// ---- convenience ----

	/// Run a script string in the root scope.
	pub fn run_script(&self, code: &str, name: &str) -> Result<(), String> {
		self.vm.run_script(code, name)
	}

	/// `this.include(path)` — load+run a script once (e.g. `"scripts/preload"`).
	pub fn include(&self, path: &str) -> Result<(), ScriptError> {
		self.modules.include(&self.vm, path)
	}

	/// Boot the state stack at `root_path` (normally `"scripts/root_state"`).
	pub fn boot(&self, root_path: &str) -> Result<SqStrongRef, String> {
		self.root_state.boot(&self.vm, root_path)
	}

	/// Advance the host-driven clocks by `real_delta_ms` (fires due Real/Virtual
	/// scheduled events).
	pub fn tick(&self, real_delta_ms: f64) {
		self.time.tick(&self.vm, real_delta_ms);
	}

	/// Run the `onInit` of states added since the last frame (see
	/// [`RootState::init_pending`]), for hosts driving a sequence between frames.
	pub fn init_pending_states(&self) -> Result<(), String> {
		self.root_state.init_pending(&self.vm)
	}

	/// Run one frame of the original's main loop with `real_delta_ms` of real time
	/// (order probed 2026-10-13): advance the clocks and fire due scheduled events, run
	/// `onUpdate` then `onRender` on every visible state, then the background task once.
	/// The sprites with a torque turn after the updates, as the scene updates.
	pub fn frame(&self, real_delta_ms: f64) -> Result<(), String> {
		self.time.tick(&self.vm, real_delta_ms);
		self.root_state.update(&self.vm)?;
		self.move_cameras(real_delta_ms);
		self.turn_sprites();
		self.audio.update((self.time.real_seconds() * 1000.0) as u32);
		self.root_state.render(&self.vm)?;
		self.root_state.run_background(&self.vm)
	}

	/// The cameras' glides and zooms, a frame of `real_delta_ms` on (see
	/// `native::camera_motion`). The world camera's screen is the video mode, and it
	/// zooms in no closer than 1 / `SceneScale` (`0x4ee9cd`).
	fn move_cameras(&self, real_delta_ms: f64) {
		let (width, height, _) = self.video.screen();
		let screen = (width as i32, height as i32);
		let scale = self.video.scene_scale() as f32;
		let zoom_min = if scale != 0.0 { 1.0 / scale } else { 1.0 };
		let dt = (real_delta_ms / 1000.0) as f32;
		let now = self.time.virtual_seconds() as f32;
		{
			let mut camera = self.world_camera.borrow_mut();
			camera.set_screen(screen);
			camera.set_zoom_min(zoom_min);
			camera.update(dt, now);
		}
		// The battle camera, when it glides.
		self.tactical_grid.move_camera(screen, zoom_min, dt, now);
	}

	/// Turn every sprite with a torque to the virtual clock (see `native::spin`).
	fn turn_sprites(&self) {
		let now = self.time.virtual_seconds() as f32;
		self.world_grid.turn(now);
		self.world_entities.turn(&self.vm, now);
		self.tactical_grid.turn(now);
		self.tactical_entities.turn(&self.vm, now);
	}

	/// Install logging to a custom sink (defaults to stderr/stdout otherwise).
	pub fn set_log_handler(&mut self, handler: impl Fn(crate::native::LogLevel, &str) + 'static) {
		crate::native::install_logging(&mut self.vm, handler);
	}
}

impl<S: ScriptSource> Drop for Engine<S> {
	fn drop(&mut self) {
		// Release every service's SqStrongRefs while the VM is still alive; the `vm`
		// field then drops last (clean `sq_close`).
		self.saves.clear();
		self.root_state.clear();
		// Flying heads hold their layers' sprites.
		self.battle_effects.borrow_mut().clear();
		self.tactical_entities.clear();
		self.tactical_navigator.clear();
		self.tactical_grid.clear();
		self.world_entities.clear();
		self.world_grid.clear();
		self.ui.clear();
		self.time.clear();
		self.modules.clear();
	}
}
