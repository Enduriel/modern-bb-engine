//! [`Engine`]: the host object that owns the VM and every stateful service, and
//! tears them down in the correct order.
//!
//! The four host services ([`ModuleSystem`], [`TimeService`], [`UiBridge`],
//! [`RootState`]) each hold [`SqStrongRef`](squirrel::SqStrongRef)s into the VM that
//! MUST be released before `sq_close` — and merely dropping a service handle is not
//! enough, because VM-resident closures keep its state alive until `sq_close`. So
//! each service has a `clear()` that releases its refs while the VM is fully alive.
//! (Refs that `sq_close` itself drops, with the objects it frees, release nothing:
//! see `squirrel`'s `vm::closing`. Clearing first keeps the release ordered and
//! leaves nothing for teardown to skip.)
//!
//! `Engine` removes that footgun: it owns all of them, and its [`Drop`] `clear()`s
//! every service *while the VM is still fully alive*, then lets the `vm` field (the
//! last-declared field) drop last for a clean `sq_close`.

mod memory;

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
/// Mixed into `NativeConfig::engine_seed` for the battle rain's own stream.
const RAIN_SEED: u64 = 0x5241_494E_0000_0001;

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
	tactical_highlighter: crate::native::TacticalHighlighter,
	tactical_rain: crate::native::tactical_rain::TacticalRain,
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
	birds: crate::native::world_birds::Birds,
	waves: crate::native::world_waves::Waves,
	/// The tactical terrain transitions (no VM handles).
	transitions: crate::native::tactical::Transitions,
	tactical_light: crate::native::tactical::TacticalLight,
	tactical_clouds: crate::native::world_clouds::Clouds,
	world_camera: crate::native::world_camera::WorldCamera,
	world_camera_callbacks: crate::native::world_camera::WorldCameraCallbacks,
	world_particles: crate::native::particles::ParticlesHandle,
	tactical_particles: crate::native::particles::ParticlesHandle,
	battle_effects: crate::native::battle_effects::BattleEffectsHandle,
	/// The achievements' and stats' service (no VM handles).
	user_stats: crate::native::user_stats::UserStatsHandle,
	/// The game's sound (no VM handles).
	audio: crate::native::audio::Audio,
	/// The keys the host reported, dispatched each frame (see [`crate::key_queue`]).
	keys: crate::key_queue::KeyQueue,
	/// The mouse events the host queued for the next frame, dispatched after its keys
	/// (see [`queue_mouse`](Self::queue_mouse)).
	mouse: RefCell<Vec<crate::input_events::MouseInput>>,
	/// The mouse cursor the scripts chose (no VM handles).
	ui_cursor: crate::native::ui_cursor::UiCursor,
	time: TimeService,
	/// The clocks a battle is drawn at (see [`crate::scene::steps`]), and what its scene
	/// holds from step to step: the clouds and the rain.
	scene_steps: crate::scene::steps::SceneSteps,
	held_clouds: RefCell<crate::scene::steps::Held<u64, Vec<crate::scene::SceneCloud>>>,
	#[allow(clippy::type_complexity)]
	held_rain: RefCell<
		crate::scene::steps::Held<
			u64,
			(Vec<crate::scene::tactical::SceneRainDrop>, Vec<crate::scene::tactical::SceneRainSplat>),
		>,
	>,
	/// Frees reference cycles where the game discards object graphs (see `collect`).
	collector: crate::collect::Collector,
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
		crate::native::stubs::reset();
		let mut vm = SQVM::new(1 << 16);
		let natives = config.install(&mut vm);

		let modules = ModuleSystem::new(source);
		modules.install(&mut vm);
		let time = TimeService::new();
		time.set_exact_clock(config.exact_clock);
		time.install(&mut vm);
		let ui = UiBridge::new();
		ui.install(&mut vm);
		let ui_cursor = crate::native::ui_cursor::UiCursor::default();
		ui_cursor.install(&mut vm, &natives.sprites);
		// After the `World` table exists (config.install), add the grid natives.
		let world_grid = WorldGrid::new();
		world_grid.set_sprites(natives.sprites.clone());
		world_grid.set_engine_rng(natives.engine_rng.clone());
		world_grid.set_camera(natives.world_camera.clone());
		world_grid.set_vec_class(natives.vec2f.clone());
		world_grid.install(&mut vm);
		// After the grid natives.
		let world_entities = WorldEntities::new(modules.instantiator());
		world_entities.set_sprites(natives.sprites.clone());
		world_entities.set_world(world_grid.access(), natives.vec2f.clone());
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
		// A tile's `clear(flags)` takes the pools and landed heads lying there too.
		{
			let effects = natives.tactical.effects.clone();
			tactical_grid.set_clear_hook(Rc::new(move |vm, tile, flags| {
				let now = crate::native::sprite_color::virtual_time(vm);
				effects.borrow_mut().clear_tile(tile, flags, now);
			}));
		}
		tactical_grid.set_engine_rng(natives.engine_rng.clone());
		tactical_grid.set_battle_camera(config.battle_camera);
		tactical_grid.install(&mut vm);
		tactical_entities.attach_map(&tactical_grid);
		let tactical_navigator = TacticalNavigator::new(&tactical_grid, tactical_entities.units());
		tactical_navigator.set_motion(config.battle_motion);
		tactical_navigator.install(&mut vm);
		// After the navigator's update and the map's clearScene, which it wraps.
		tactical_entities.install_shaker(&mut vm);
		let tactical_highlighter = crate::native::TacticalHighlighter::new(&tactical_grid, tactical_entities.units());
		tactical_highlighter.install(&mut vm);
		crate::native::tactical::install_effects(&mut vm, &natives.tactical);
		// The battle's rain, last in `Tactical.update` (after the clouds), on its own stream.
		let tactical_rain = crate::native::tactical_rain::TacticalRain::new(
			&tactical_grid,
			natives.video.clone(),
			natives.sprites.clone(),
			config.engine_seed.map_or_else(crate::native::EngineRng::unseeded, |s| crate::native::EngineRng::seeded(s ^ RAIN_SEED)),
		);
		tactical_rain.install(&mut vm);
		// Leaving a campaign empties the world (after every `World` native exists).
		crate::native::world_clear_scene::install(
			&mut vm,
			crate::native::world_clear_scene::WorldScene {
				entities: world_entities.clone(),
				grid: world_grid.clone(),
				units: tactical_entities.clone(),
				time: time.clone(),
				clouds: natives.clouds.clone(),
				birds: natives.birds.clone(),
				waves: natives.waves.clone(),
				particles: natives.world_particles.clone(),
			},
		);
		// Silent until the host gives it an output; its own random stream, as the original's.
		let audio = crate::native::audio::Audio::new(
			Box::new(crate::native::audio::NullOut::new(modules.file_reader())),
			config.engine_seed.map_or_else(crate::native::EngineRng::unseeded, |s| crate::native::EngineRng::seeded(s ^ AUDIO_SEED)),
			time.clone(),
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
		// After the natives it wraps (the clearScenes and World.load).
		let collector = crate::collect::Collector::install(&mut vm, world_entities.populated_reader());
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
			tactical_highlighter,
			tactical_rain,
			ui,
			ui_client: RefCell::new(crate::ui_client::UiClient::new()),
			sprites: natives.sprites,
			video: natives.video,
			world_clock: natives.world_clock.clone(),
			clouds: natives.clouds.clone(),
			birds: natives.birds.clone(),
			waves: natives.waves.clone(),
			transitions: natives.tactical.transitions.clone(),
			tactical_light: natives.tactical.light.clone(),
			tactical_clouds: natives.tactical.clouds.clone(),
			world_camera: natives.world_camera.clone(),
			world_camera_callbacks: natives.world_camera_callbacks.clone(),
			world_particles: natives.world_particles.clone(),
			tactical_particles: natives.tactical.particles.clone(),
			battle_effects: natives.tactical.effects.clone(),
			user_stats: natives.user_stats.clone(),
			audio,
			keys: Default::default(),
			mouse: Default::default(),
			ui_cursor,
			time,
			scene_steps: Default::default(),
			held_clouds: Default::default(),
			held_rain: Default::default(),
			collector,
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
	/// The stub natives the scripts called since this engine was made, with their call
	/// counts, by name (see `native::stubs`): what the scripts ask for that this engine
	/// doesn't do yet.
	pub fn stub_calls(&self) -> Vec<(String, u64)> {
		crate::native::stubs::calls()
	}
	/// The achievements' and stats' service the natives call: the host sets Steam's
	/// there (the default, [`OfflineStats`](crate::native::user_stats::OfflineStats),
	/// unlocks nothing).
	pub fn user_stats(&self) -> &crate::native::user_stats::UserStatsHandle {
		&self.user_stats
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

	/// Give the scripts' `Settings` what the host's settings file holds besides the video
	/// mode, as the original reads its config file at start; `Settings.load` reads it
	/// back from here (with the video mode as it is now). Before the scripts boot.
	pub fn apply_game_settings(&self, game: &crate::GameSettings) -> Result<(), String> {
		crate::native::settings_apply(&self.vm, game)?;
		self.video.set_stored(crate::SavedSettings { video: self.video.mode(), game: *game });
		Ok(())
	}

	/// The settings the scripts' `Settings` holds now, besides the video mode.
	pub fn game_settings(&self) -> crate::GameSettings {
		crate::native::settings_read(&self.vm)
	}
	/// The sprite registry behind `doesBrushExist`; the host fills it at startup.
	pub fn sprites(&self) -> &Sprites {
		&self.sprites
	}
	/// The world map as the scripts have set it up, for a renderer. The fog of war's
	/// mask comes separately, as patches ([`WorldGrid::take_fog_patch`]).
	pub fn world_scene(&self) -> crate::scene::WorldScene {
		self.world_scene_where(|_| true)
	}

	/// [`world_scene`](Self::world_scene) with only the tiles whose centre `keep_tile`
	/// accepts (in world units), e.g. those the screen can show: a frame then doesn't copy
	/// the whole map. Everything else is whole.
	pub fn world_scene_where(&self, keep_tile: impl Fn((f64, f64)) -> bool) -> crate::scene::WorldScene {
		crate::scene::WorldScene {
			tiles: self.world_grid.scene_tiles_where(keep_tile),
			entities: self.world_entities.scene_entities(&self.vm),
			extent: self.world_grid.extent(),
			fog_of_war: self.world_grid.fog_of_war(),
			region_texts: self.world_grid.region_texts(),
			light: {
				let (color, saturation) = self.world_clock.borrow().ambient();
				crate::scene::SceneLight { color, saturation }
			},
			clouds: self.clouds.borrow().clouds().to_vec(),
			birds: self.birds.borrow().birds(),
			footprints: self.world_entities.scene_footprints(&self.vm),
			waves: self.waves.borrow().scene(),
			camera: self.world_camera(),
			particles: self.world_particles.borrow().scene(),
		}
	}

	/// What [`world_scene_where`](Self::world_scene_where) builds the scene from besides
	/// its tiles, without building it: two frames with equal keys have equal scenes. Its
	/// count of changes moves whenever something the scene shows was set or moved on its
	/// own (see `world_scene_changes`); the rest is compared as it is.
	pub fn world_scene_key(&self) -> crate::scene::WorldSceneKey {
		let extent = self.world_grid.extent();
		crate::scene::WorldSceneKey {
			changes: crate::native::world_scene_changes::count(),
			extent,
			fog_of_war: self.world_grid.fog_of_war(),
			light: {
				let (color, saturation) = self.world_clock.borrow().ambient();
				crate::scene::SceneLight { color, saturation }
			},
			camera: self.world_camera(),
			tracking: crate::native::WorldEntities::tracking(&self.vm),
			// Without a map (the main menu) there are no footprints to fade: the clock
			// running on there changes nothing drawn.
			virtual_time: if extent.0 > 0.0 && extent.1 > 0.0 { self.time.virtual_seconds() } else { 0.0 },
		}
	}

	/// The world camera (`World.getCamera()`): where the screen's centre is and its zoom.
	pub fn world_camera(&self) -> crate::scene::SceneWorldCamera {
		crate::native::world_camera::view(&self.world_camera)
	}
	/// What the portrait of entity `id` of `kind` draws: a battle unit's (placed on the
	/// map or not, e.g. a brother in the roster) or a world entity's. `None` when no
	/// live entity of that kind has the id: each kind finds only its own, as the
	/// original's `tacticalentity` handler finds only battle units.
	pub fn portrait(&self, kind: crate::scene::PortraitKind, id: i64) -> Option<crate::scene::ScenePortrait> {
		match kind {
			crate::scene::PortraitKind::Tactical => self.tactical_entities.portrait(&self.vm, id),
			crate::scene::PortraitKind::World => self.world_entities.portrait(&self.vm, id),
		}
	}

	/// The fonts mods registered (`ModernEngine.registerFont`), from the `from`th on
	/// (0-based), in registration order: each consumer (the renderer, the UI page)
	/// counts what it has taken.
	pub fn font_registrations(&self, from: usize) -> Vec<std::rc::Rc<crate::modern_engine::FontRegistration>> {
		self.modules.font_registrations(from)
	}

	/// The scenes the game discarded since the last call (a battle's end, the campaign
	/// left, a save loaded), in order, for the host to drop what they drew.
	pub fn take_discards(&self) -> Vec<crate::Discard> {
		self.collector.take_discards()
	}

	/// Whether a battle map is set up (from its building to `Tactical.clearScene`).
	pub fn has_battle_map(&self) -> bool {
		self.tactical_grid.has_map()
	}

	/// The battle map as the scripts have set it up, for a renderer.
	/// Mirror the battle camera's view, or not: its `IsFlipped`, which scripts can only
	/// read (the original's engine flips it).
	pub fn set_battle_camera_flipped(&self, flipped: bool) {
		self.tactical_grid.set_camera_flipped(flipped);
	}

	/// Draw battles in steps (see [`crate::scene::steps`]) for a frame of `fit` pixels to
	/// a pixel of the game's screen, or (`None`, the default) at the clocks as they are.
	pub fn set_scene_steps(&self, fit: Option<f32>) {
		self.scene_steps.set_fit(fit);
	}

	/// The step battles are drawn in now, in doublings of 1/60 s (see
	/// [`crate::scene::steps`]), while they are.
	pub fn scene_step_doublings(&self) -> Option<u32> {
		self.scene_steps.on().then(|| self.scene_steps.doublings_now())
	}

	/// The battle map as the scripts have set it up, for a renderer: what moves with time
	/// as it is at the scene's clock (see [`crate::scene::steps`]).
	pub fn tactical_scene(&self) -> crate::scene::tactical::TacticalScene {
		self.tactical_scene_in_steps(true)
	}

	/// The battle map at the clocks as they are, whether battles are drawn in steps or
	/// not (what [`tactical_scene`](Self::tactical_scene) shows at a step). For tests.
	pub fn tactical_scene_as_is(&self) -> crate::scene::tactical::TacticalScene {
		self.tactical_scene_in_steps(false)
	}

	/// The number of steps the scene's clocks were taken at (see
	/// [`crate::scene::steps`]): a new one each frame while battles aren't drawn in steps.
	pub fn scene_steps_taken(&self) -> u64 {
		self.scene_steps.generation()
	}

	fn tactical_scene_in_steps(&self, in_steps: bool) -> crate::scene::tactical::TacticalScene {
		use crate::native::sprite_color::real_time;
		let steps = &self.scene_steps;
		let (on, generation) = (in_steps && steps.on(), steps.generation());
		let held = if on { steps.clock(self.clocks()) } else { self.clocks() };
		// Off, the clocks as the scripts read them.
		let real = if on { held.real as f32 } else { real_time(&self.vm) };
		let speed = self.time.virtual_speed() as f32;
		let (raining, rain_generation) = self.tactical_rain.state();
		if raining {
			steps.need_finest();
		}
		let (rain, rain_splats) =
			self.held_rain.borrow_mut().get(on, generation, rain_generation, || self.tactical_rain.scene(real));
		let clouds = {
			let cover = self.tactical_clouds.borrow();
			// They drift with the real clock while the game's runs.
			steps.need(if speed != 0.0 { cover.fastest() } else { 0.0 });
			self.held_clouds.borrow_mut().get(on, generation, cover.generation(), || cover.clouds().to_vec())
		};
		let (effects, pools) = self
			.battle_effects
			.borrow_mut()
			.scene(
				self.time.virtual_seconds() as f32,
				|tile| self.tactical_grid.tile_world_pos(tile),
				|sprite| self.tactical_entities.sprite_look(&self.vm, sprite),
			);
		let mut scene = crate::scene::tactical::TacticalScene {
			effects,
			pools,
			rain,
			rain_splats,
			transitions: self.transitions.borrow().clone(),
			entities: self.tactical_entities.scene_entities(
				&self.vm,
				real,
				on.then_some((held.virtual_secs as f32, speed)),
				steps,
			),
			light: {
				let (color, saturation) = self.tactical_light.get();
				crate::scene::SceneLight { color, saturation }
			},
			clouds,
			hide_trees: crate::native::temp_gameplay_flag(&self.vm, "HideTrees"),
			particles: self.tactical_particles.borrow().scene(),
			overlay_icons: self.tactical_highlighter.scene_icons(real, steps),
			path: self.tactical_navigator.scene_path(),
			blocked_tiles: self.tactical_highlighter.scene_blocked(),
			..self.tactical_grid.scene(real, on, steps)
		};
		crate::native::tactical_unveil::mark(
			&mut scene,
			&self.tactical_entities.occlusion_preventers(),
			&self.tactical_grid.tiles_with_items(),
		);
		scene
	}
	/// Let the headless UI client answer what the scripts asked of the UI since the
	/// last pump (see [`crate::ui_client::UiClient::pump`]).
	pub fn pump_ui(&self) -> Result<Vec<crate::UiCall>, String> {
		self.ui_client.borrow_mut().pump(&self.ui, &self.vm)
	}

	/// What a script error does to the engine's calls into the scripts from now on: the
	/// states' hooks and the headless UI's replies (see [`crate::ScriptErrors`]). A game a
	/// player plays goes on, as the original; with [`ScriptErrors::GoOn`](crate::ScriptErrors::GoOn)
	/// install a reporter first ([`crate::report_script_errors`]), or the errors go unseen.
	pub fn set_script_errors(&self, policy: crate::ScriptErrors) {
		self.root_state.set_script_errors(policy);
		self.ui_client.borrow_mut().set_script_errors(policy);
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

	/// A key went down or up (the system's repeats may come in too): the states get it
	/// from the next frame, and a held key every frame until it's released, as the
	/// original's input manager does (see [`crate::key_queue`]).
	pub fn queue_key(&self, key: i64, down: bool, modifier: i64) {
		self.keys.push(key, down, modifier);
	}

	/// The window got the keyboard or lost it: the queued and held keys are forgotten,
	/// none released, and input that comes while it lacks it is dropped, as the
	/// original's main loop does (see [`crate::key_queue`]).
	pub fn keyboard_focus(&self, focused: bool) {
		self.keys.set_focused(focused);
	}

	/// A mouse event for the states at the next frame, after its keys, as the original's
	/// input manager dispatches its queue once a frame: the keys, then the mouse's
	/// events in order (`0x551550`, disassembled 2026-10-23). A host merges the pointer's
	/// moves into one a frame (the original's system merges them to a few, probed
	/// 2026-10-23) and delivers a move before a click at once, so the click comes after
	/// the move to where it happened.
	pub fn queue_mouse(&self, input: crate::input_events::MouseInput) {
		self.mouse.borrow_mut().push(input);
	}

	/// A key typed `c` with the `modifier` mask held (each of the system's repeats
	/// again): the UI gets it from the next frame, after the key (see [`queue_key`]).
	///
	/// [`queue_key`]: Self::queue_key
	pub fn queue_text(&self, c: char, modifier: i64) {
		self.keys.push_text(c, modifier);
	}

	/// The mouse cursor to show: the sprite the scripts chose, and where they last put
	/// it (`UI.setCursorPosition`, screen pixels), see [`crate::native::ui_cursor`].
	pub fn cursor(&self) -> (crate::CursorLook, (i64, i64)) {
		(self.ui_cursor.look(), self.ui.cursor())
	}

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
	/// The queued keys reach the states after the clocks, as the original dispatches
	/// its input before the scene (held keys again every frame, see [`queue_key`]),
	/// then the queued mouse events ([`queue_mouse`](Self::queue_mouse)).
	/// The sprites with a torque turn after the updates, as the scene updates.
	///
	/// [`queue_key`]: Self::queue_key
	pub fn frame(&self, real_delta_ms: f64) -> Result<(), String> {
		let keys = self.begin_frame(real_delta_ms);
		self.finish_frame(real_delta_ms, &keys)
	}

	/// A [`frame`](Self::frame)'s start: the clocks advance and due scheduled events
	/// fire. Gives the frame's key events, which [`finish_frame`](Self::finish_frame)
	/// dispatches to the states. Between the two a UI host hands them to its page: the
	/// original's UI listener gets each key event before the states (`0x56cb40`, probed
	/// 2026-10-21), a held key's press each frame, and what keys type (which the states
	/// never get).
	pub fn begin_frame(&self, real_delta_ms: f64) -> Vec<crate::key_queue::QueuedInput> {
		let _clock = crate::frame_timing::span("engine clock+keys");
		self.time.tick(&self.vm, real_delta_ms);
		// The battle's scene clock steps (see `scene::steps`).
		let zoom = self.tactical_grid.camera_zoom();
		if self.scene_steps.sample(self.clocks(), if zoom > 0.0 { (1.0 / zoom) as f32 } else { 1.0 }) {
			self.tactical_entities.scene_stepped();
		}
		self.keys.take_frame()
	}

	/// The real and virtual clocks now.
	fn clocks(&self) -> crate::scene::steps::HeldClock {
		crate::scene::steps::HeldClock {
			real: self.time.real_seconds(),
			virtual_secs: self.time.virtual_seconds(),
		}
	}

	/// The rest of a frame begun with [`begin_frame`](Self::begin_frame): `keys` to the
	/// states, then the queued mouse events, then the scene.
	pub fn finish_frame(&self, real_delta_ms: f64, keys: &[crate::key_queue::QueuedInput]) -> Result<(), String> {
		use crate::frame_timing::span;
		let clock = span("engine clock+keys");
		for input in keys {
			if let crate::key_queue::QueuedInput::Key(key) = input {
				self.root_state.key_input(&self.vm, *key)?;
			}
		}
		drop(clock);
		let mouse = std::mem::take(&mut *self.mouse.borrow_mut());
		if !mouse.is_empty() {
			let _scripts = span("input scripts");
			for input in mouse {
				self.root_state.mouse_input(&self.vm, input)?;
			}
		}
		let update = span("scripts update");
		self.root_state.update(&self.vm)?;
		drop(update);
		let cameras = span("cameras");
		self.move_cameras(real_delta_ms);
		drop(cameras);
		let turn = span("sprites turn");
		self.turn_sprites();
		drop(turn);
		let audio = span("audio update");
		self.audio.update();
		drop(audio);
		self.tactical_grid.begin_frame();
		let render = span("scripts render");
		self.root_state.render(&self.vm)?;
		drop(render);
		let background = span("scripts background");
		self.root_state.run_background(&self.vm)?;
		drop(background);
		// What a battle's end, leaving a campaign or a load discarded this frame.
		let _collect = span("collect");
		if self.collector.collect_if_requested(&self.vm) {
			self.audio.discard_unused();
		}
		Ok(())
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

	/// Turn every sprite with a torque, and step every animated one, to the virtual
	/// clock (see `native::spin`, `native::sprite_anim`).
	fn turn_sprites(&self) {
		let secs = self.time.virtual_seconds();
		let (now, now_ms) = (secs as f32, (secs * 1000.0) as u32);
		let real = self.time.real_seconds() as f32;
		self.world_grid.turn(now, now_ms, real);
		self.world_entities.turn(&self.vm, now, now_ms);
		// The details' turns are drawn in the scene's steps (see `scene::steps`).
		let steps = &self.scene_steps;
		let fastest = self.tactical_grid.turn(now, now_ms, real, steps.stepped(), steps.on());
		steps.need(fastest * self.time.virtual_speed() as f32);
		self.tactical_entities.turn(&self.vm, now, now_ms);
	}

	/// The sprites that may change on their own (a torque, frames to play, a fade) but
	/// that the frame's sprite passes wouldn't visit, as their owner isn't noted as
	/// moving (see `native::moving_sprites`): always 0 unless a way a sprite starts
	/// moving was missed. For tests.
	pub fn unnoted_moving_sprites(&self) -> usize {
		self.world_entities.unnoted_moving(&self.vm)
			+ self.tactical_entities.unnoted_moving(&self.vm)
			+ self.tactical_grid.unnoted_moving()
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
		self.tactical_highlighter.clear();
		self.tactical_rain.clear();
		self.tactical_grid.clear();
		self.world_entities.clear();
		self.world_grid.clear();
		self.world_camera_callbacks.borrow_mut().clear();
		self.ui.clear();
		self.time.clear();
		self.modules.clear();
	}
}
