//! The game behind the window: the engine, booted into a new campaign, stepped by the
//! frame's real time, fed the player's input, and drawn by the renderer into a frame
//! the window presents. The scripts' UI is the [`Front`]'s: headless, or with the `ui`
//! feature the real page drawn over the frame.

use std::path::Path;
use std::sync::Arc;

use bb_engine::files::GameFiles;
use bb_engine::harness::{self, GameEngine};
use bb_render::portrait::PortraitRequest;
use bb_render::world::WorldView;
use bb_render::{Atlases, Clock, Gpu, RenderTarget, WorldRenderer, write_png};
use squirrel::{SqStrongRef, SqTable};
use squirrel_bb::{KeyInput, MouseInput};

use crate::front::{Front, Key};

/// The longest real time one frame advances the game by.
const MAX_FRAME_MS: f64 = 250.0;

pub struct Game {
	front: Front,
	renderer: WorldRenderer,
	/// The screen the scripts think in (the video mode); window pixels scale to it.
	screen: (u32, u32),
	cursor: (i64, i64),
	/// The modifier keys held (`KeyModifier` mask), sent with every event.
	modifier: i64,
}

impl Game {
	/// Boot the engine from the install at `data_dir` into a new campaign on `seed`.
	pub fn boot(gpu: &Gpu, data_dir: &Path, seed: &str) -> Result<Game, String> {
		let io = |e: std::io::Error| e.to_string();
		let front = Front::boot(gpu, data_dir, seed)?;
		let files = GameFiles::open(harness::data_archives(data_dir).map_err(io)?).map_err(io)?;
		let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
		let renderer = WorldRenderer::new(gpu, Arc::new(files), threads);
		let mode = video_mode(front.engine()).unwrap_or((1920, 1080));
		Ok(Game {
			front,
			renderer,
			screen: mode,
			cursor: (0, 0),
			modifier: 0,
		})
	}

	/// The scripts and their UI.
	pub fn front(&self) -> &Front {
		&self.front
	}

	fn engine(&self) -> &GameEngine {
		self.front.engine()
	}

	/// The size the game's screen is (its video mode).
	pub fn screen(&self) -> (u32, u32) {
		self.screen
	}

	/// Advance the game by `ms` of real time.
	pub fn step(&mut self, ms: f64) -> Result<(), String> {
		self.front.step(ms.clamp(0.0, MAX_FRAME_MS))
	}

	/// The cursor moved to `(x, y)` in the game's screen pixels. The states see every
	/// move (see [`crate::front`]).
	pub fn mouse_moved(&mut self, x: i64, y: i64) -> Result<(), String> {
		self.cursor = (x, y);
		self.front.cursor(x, y);
		// Every pointer move is `MouseState.Moved`, button held or not (disassembled).
		self.mouse(MouseInput {
			id: 6,
			state: 2,
			x,
			y,
			modifier: self.modifier,
		})
	}

	/// Button `id` (1 left, 2 right, 3 middle, 4 back, 5 forward) went down or up: to
	/// the UI, or to the states.
	pub fn mouse_button(&mut self, id: i64, down: bool) -> Result<(), String> {
		let (x, y) = self.cursor;
		if self.front.button(x, y, id, down) {
			return Ok(());
		}
		self.mouse(MouseInput {
			id,
			state: if down { 0 } else { 1 },
			x,
			y,
			modifier: self.modifier,
		})
	}

	/// The wheel turned a notch, `up` or down: for the UI, or the states.
	pub fn wheel(&mut self, up: bool) -> Result<(), String> {
		let (x, y) = self.cursor;
		if self.front.wheel(x, y, up) {
			return Ok(());
		}
		self.mouse(MouseInput {
			id: 7,
			state: if up { 3 } else { 4 },
			x,
			y,
			modifier: self.modifier,
		})
	}

	/// The modifier keys held now (see `keys::modifier`).
	pub fn set_modifier(&mut self, modifier: i64) {
		self.modifier = modifier;
	}

	/// A key went down or up: to the UI, and to the states unless the UI keeps it. A
	/// held key's repeats reach the states with `getFire()` 1 (`KeyFire` Repeated).
	pub fn key(&mut self, key: &Key) -> Result<(), String> {
		if !self.front.key(key, self.modifier) {
			return Ok(());
		}
		let Some(code) = key.engine else { return Ok(()) };
		let input = KeyInput {
			key: code,
			state: i64::from(key.down),
			modifier: self.modifier,
			fire: i64::from(key.repeat),
		};
		self.engine().root_state().key_input(self.engine().vm(), input).map(|_| ())
	}

	fn mouse(&mut self, input: MouseInput) -> Result<(), String> {
		self.engine().root_state().mouse_input(self.engine().vm(), input).map(|_| ())
	}

	/// Draw what the player sees now into `target`.
	pub fn draw(&mut self, gpu: &Gpu, target: &RenderTarget) {
		let engine = self.front.engine();
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
		// The UI over everything, with the portraits it asked for.
		let renderer = &mut self.renderer;
		self.front
			.serve_procedural(|engine, path| procedural_png(renderer, gpu, engine, path));
		self.front.draw(gpu, target);
	}
}

/// The PNG for the page's `coui://procedural/<path>` (an entity's portrait, e.g.
/// `tacticalentity(...)`), or `None` for the transparent placeholder: an unknown path
/// or entity, or nothing to draw. Like the original's, its pixels are as drawn
/// (colour premultiplied), not unpremultiplied for the PNG.
fn procedural_png(renderer: &mut WorldRenderer, gpu: &Gpu, engine: &GameEngine, path: &str) -> Option<Vec<u8>> {
	let request = PortraitRequest::parse(path)?;
	let portrait = engine.portrait(request.id)?;
	let image = engine
		.sprites()
		.with(|registry| renderer.compose_portrait(gpu, &portrait, &request.hidden, registry));
	let image = match image {
		Ok(image) => image?,
		Err(e) => {
			eprintln!("portrait {path}: {e}");
			return None;
		}
	};
	let mut png = Vec::new();
	write_png(&mut png, &image).ok()?;
	Some(png)
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
