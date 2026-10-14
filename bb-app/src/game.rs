//! The game behind the window: the engine, booted into a new campaign, stepped by the
//! frame's real time, fed the player's input, and drawn by the renderer into a frame
//! the window presents.
//!
//! The UI isn't drawn yet (it needs the browser host composited in), so its screens run
//! headless and the cursor is never over the UI. That is also why the campaign's
//! opening event is closed at the start: nothing could click it away.

use std::path::Path;
use std::sync::Arc;

use bb_engine::files::GameFiles;
use bb_engine::harness::{self, GameEngine};
use bb_render::world::WorldView;
use bb_render::{Atlases, Clock, Gpu, RenderTarget, WorldRenderer};
use squirrel::{SqStrongRef, SqTable};
use squirrel_bb::{KeyInput, MouseInput};

/// The longest real time one frame advances the game by.
const MAX_FRAME_MS: f64 = 250.0;

pub struct Game {
	engine: GameEngine,
	renderer: WorldRenderer,
	/// The screen the scripts think in (the video mode); window pixels scale to it.
	screen: (u32, u32),
	cursor: (i64, i64),
	/// Mouse buttons held, for "moved with a button down".
	buttons_down: u32,
}

impl Game {
	/// Boot the engine from the install at `data_dir` into a new campaign on `seed`.
	pub fn boot(gpu: &Gpu, data_dir: &Path, seed: &str) -> Result<Game, String> {
		let io = |e: std::io::Error| e.to_string();
		let engine = harness::open_install(data_dir).map_err(io)?;
		harness::boot_menu(&engine)?;
		harness::prepare_new_campaign(&engine, seed)?;
		harness::start_new_campaign(&engine)?;
		engine.run_script("::World.State.m.MenuStack.popAll(true);", "close the opening event")?;
		let files = GameFiles::open(harness::data_archives(data_dir).map_err(io)?).map_err(io)?;
		let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
		let renderer = WorldRenderer::new(gpu, Arc::new(files), threads);
		let mode = video_mode(&engine).unwrap_or((1920, 1080));
		Ok(Game {
			engine,
			renderer,
			screen: mode,
			cursor: (0, 0),
			buttons_down: 0,
		})
	}

	/// The size the game's screen is (its video mode).
	pub fn screen(&self) -> (u32, u32) {
		self.screen
	}

	/// Advance the game by `ms` of real time.
	pub fn step(&mut self, ms: f64) -> Result<(), String> {
		self.engine.frame(ms.clamp(0.0, MAX_FRAME_MS))?;
		self.engine.pump_ui()?;
		Ok(())
	}

	/// The cursor moved to `(x, y)` in the game's screen pixels.
	pub fn mouse_moved(&mut self, x: i64, y: i64) -> Result<(), String> {
		self.cursor = (x, y);
		self.engine.ui().set_cursor(x, y, false);
		let state = if self.buttons_down != 0 { 2 } else { 0 };
		self.mouse(MouseInput { id: 6, state, x, y })
	}

	/// Button `id` (1 left, 2 right, 3 middle, 4 back, 5 forward) went down or up.
	pub fn mouse_button(&mut self, id: i64, down: bool) -> Result<(), String> {
		let bit = 1 << id;
		if down {
			self.buttons_down |= bit;
		} else {
			self.buttons_down &= !bit;
		}
		let (x, y) = self.cursor;
		self.mouse(MouseInput {
			id,
			state: if down { 0 } else { 1 },
			x,
			y,
		})
	}

	/// The wheel turned a notch, `up` or down.
	pub fn wheel(&mut self, up: bool) -> Result<(), String> {
		let (x, y) = self.cursor;
		self.mouse(MouseInput {
			id: 7,
			state: if up { 3 } else { 4 },
			x,
			y,
		})
	}

	/// Key `key` (an engine code, see `keys`) went down or up with `modifier` held.
	pub fn key(&mut self, key: i64, down: bool, modifier: i64) -> Result<(), String> {
		let input = KeyInput {
			key,
			state: i64::from(down),
			modifier,
		};
		self.engine.root_state().key_input(self.engine.vm(), input).map(|_| ())
	}

	fn mouse(&mut self, input: MouseInput) -> Result<(), String> {
		self.engine.root_state().mouse_input(self.engine.vm(), input).map(|_| ())
	}

	/// Draw what the player sees now into `target`.
	pub fn draw(&mut self, gpu: &Gpu, target: &RenderTarget) {
		let engine = &self.engine;
		self.renderer.prefetch(engine.sprites().take_hints());
		self.renderer.set_clock(Clock {
			real: engine.time().real_seconds() as f32,
			virtual_time: engine.time().virtual_seconds() as f32,
		});
		let (width, height) = (target.width, target.height);
		// The screen the scripts think in maps onto the window.
		let fit = width as f32 / self.screen.0 as f32;
		if tactical_visible(engine) {
			let scene = engine.tactical_scene();
			let view = WorldView {
				center: [scene.camera.pos.0 as f32, scene.camera.pos.1 as f32],
				zoom: fit / scene.camera.zoom as f32,
				width,
				height,
			};
			engine.sprites().with(|registry| {
				self.renderer.draw_tactical(gpu, target, &scene, registry, &view, Atlases::DrawReady)
			});
		} else {
			if let Some(patch) = engine.world_grid().take_fog_patch() {
				self.renderer.update_fog(gpu, &patch);
			}
			let scene = engine.world_scene();
			let view = WorldView {
				center: [scene.camera.pos.0, scene.camera.pos.1],
				zoom: fit / scene.camera.zoom,
				width,
				height,
			};
			engine.sprites().with(|registry| {
				self.renderer.draw(gpu, target, &scene, registry, &view, Atlases::DrawReady)
			});
		}
	}
}

/// `Settings.getVideoMode()`.
fn video_mode(engine: &GameEngine) -> Option<(u32, u32)> {
	let vm = engine.vm();
	let settings = SqTable::root(vm).get::<SqTable>("Settings")?;
	let mode = settings.get::<SqStrongRef>("getVideoMode")?.call_ret::<SqStrongRef, _>(vm, ()).ok()??;
	let dim = |k: &str| mode.get_member::<squirrel::Number>(vm, k).map(|n| n.0 as u32);
	Some((dim("Width")?, dim("Height")?))
}

/// `Tactical.isVisible()`: a battle is showing.
fn tactical_visible(engine: &GameEngine) -> bool {
	let vm = engine.vm();
	SqTable::root(vm)
		.get::<SqTable>("Tactical")
		.and_then(|t| t.get::<SqStrongRef>("isVisible"))
		.and_then(|f| f.call_ret::<bool, _>(vm, ()).ok().flatten())
		.unwrap_or(false)
}
