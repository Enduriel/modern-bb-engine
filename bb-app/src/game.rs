//! The game behind the window: the engine, booted to the main menu or into a new
//! campaign, stepped by the frame's real time, fed the player's input, and drawn by the
//! renderer into a frame the window presents. The scripts' UI is the [`Front`]'s: headless, or with the `ui`
//! feature the real page drawn over the frame.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use bb_engine::files::GameFiles;
use bb_engine::harness::{self, GameEngine};
use bb_render::portrait::PortraitRequest;
use bb_render::world::WorldView;
use bb_render::{Atlases, Clock, Discard, FrameStats, Gpu, RenderTarget, WorldRenderer, write_png};
use squirrel::{SqStrongRef, SqTable};
use squirrel_bb::{MouseInput, VideoMode, VideoSettings};

use crate::front::{Front, Key};

/// The longest real time one frame advances the game by.
const MAX_FRAME_MS: f64 = 250.0;

pub struct Game {
	front: Front,
	renderer: WorldRenderer,
	/// The battle sprites' atlases hinted while no battle map was set up (the company's
	/// units on the world map), warmed whenever one is; and whether one was last frame.
	battle_atlases: HashSet<String>,
	battle_map: bool,
	/// How many of the fonts mods registered the renderer has.
	fonts: usize,
	cursor: (i64, i64),
	/// The modifier keys held (`KeyModifier` mask), sent with every event.
	modifier: i64,
}

/// Where the game starts.
#[derive(Clone, Debug)]
pub enum Start {
	/// The main menu, where the player starts or loads a campaign.
	Menu,
	/// A new campaign on this map seed, started as the main menu's Start does.
	NewCampaign(String),
}

/// How [`Game::start`] boots the game.
#[derive(Clone, Debug)]
pub struct Boot<'a> {
	/// The install's data folder.
	pub data_dir: &'a Path,
	pub start: Start,
	/// The install's mods mounted over its archives ([`archives`]), for the scripts,
	/// the page, the renderer and the sound alike.
	pub mods: bool,
	/// The sound on the system's output device; silent otherwise (as for tests).
	pub sound: bool,
	/// The world map's columns and rows for the campaigns started; vanilla's otherwise.
	pub world_size: Option<(i64, i64)>,
	pub mode: VideoMode,
	/// Serve Chrome DevTools for the page at this port (the page's front only).
	pub devtools: Option<u16>,
}

/// The archives the game reads from the install at `data_dir`: its `data_*.dat`, then,
/// with `mods`, its mods' zips over them, as the original mounts them.
pub fn archives(data_dir: &Path, mods: bool) -> std::io::Result<Vec<std::path::PathBuf>> {
	let mut archives = harness::data_archives(data_dir)?;
	if mods {
		archives.extend(harness::mod_archives(data_dir)?.iter().map(|p| harness::locate_mod(p)));
	}
	Ok(archives)
}

impl Game {
	/// Boot the engine from the install at `data_dir` into a new campaign on `seed`, in
	/// video `mode`, playing its sound on the device if `sound` (silent otherwise, as for
	/// tests), with DevTools for the page at port `devtools` (the page's front only).
	/// Without the install's mods; [`Game::start`] says more.
	pub fn boot(
		gpu: &Gpu,
		data_dir: &Path,
		seed: &str,
		sound: bool,
		world_size: Option<(i64, i64)>,
		mode: VideoMode,
		devtools: Option<u16>,
	) -> Result<Game, String> {
		let boot = Boot {
			data_dir,
			start: Start::NewCampaign(seed.to_string()),
			mods: false,
			sound,
			world_size,
			mode,
			devtools,
		};
		Game::start(gpu, &boot)
	}

	/// Boot the engine as `boot` says: to the main menu, or on into a new campaign.
	pub fn start(gpu: &Gpu, boot: &Boot) -> Result<Game, String> {
		let io = |e: std::io::Error| e.to_string();
		let archives = archives(boot.data_dir, boot.mods).map_err(io)?;
		let files = GameFiles::open(archives.clone()).map_err(io)?;
		let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
		let mut renderer = WorldRenderer::new(gpu, Arc::new(files), threads);
		// The game's brushes (a few milliseconds) before the boot, so a first run's sheets
		// are built on spare workers while worldgen runs (see `bb_render`'s pages).
		let mut dats: Vec<bb_engine::DatArchive> = archives.iter().filter_map(|p| bb_engine::DatArchive::open(p).ok()).collect();
		renderer.prefetch(&bb_engine::assets::load_sprites(&mut dats).registry, []);
		let front = Front::boot(gpu, boot)?;
		let mut game = Game {
			front,
			renderer,
			battle_atlases: HashSet::new(),
			fonts: 0,
			battle_map: false,
			cursor: (0, 0),
			modifier: 0,
		};
		// What the boot hinted (worldgen's brushes) starts decoding before the first frame.
		game.follow_scenes();
		Ok(game)
	}

	/// Keep the campaign's saves as files in `dir` (made when first saved to), rather than
	/// in memory. Their Save and Load menus read it when they open.
	pub fn keep_saves_in(&self, dir: std::path::PathBuf) {
		self.engine().saves().set_directory(dir);
	}

	/// The scripts and their UI.
	pub fn front(&self) -> &Front {
		&self.front
	}

	/// Whether the scripts have quit: their last state finished (the main menu's Quit
	/// does that, and Escape on the main menu), which ends the original.
	pub fn quit(&self) -> bool {
		self.engine().root_state().state_names().is_empty()
	}

	fn engine(&self) -> &GameEngine {
		self.front.engine()
	}

	/// The video mode the game runs in, shared with the scripts.
	pub fn video(&self) -> &VideoSettings {
		self.engine().video()
	}

	/// The size the game's screen is (its video mode's), which window pixels scale to.
	pub fn screen(&self) -> (u32, u32) {
		let (width, height, _) = self.video().screen();
		(width.max(1) as u32, height.max(1) as u32)
	}

	/// Run the Squirrel `code`.
	pub fn run_script(&self, code: &str) -> Result<(), String> {
		self.engine().run_script(code, "script")
	}

	/// The bytes of textures kept on the GPU before the least recently drawn go (see
	/// `bb_render::residency`).
	pub fn set_texture_budget(&mut self, bytes: u64) {
		self.renderer.set_texture_budget(bytes);
	}

	/// Track which parts of the atlases are drawn from now on (see
	/// [`bb_render::WorldRenderer::track_usage`]).
	pub fn track_texture_usage(&mut self) {
		self.renderer.track_usage();
	}

	/// The atlases' use since tracking started.
	pub fn texture_usage(&self) -> Option<Vec<bb_render::AtlasUse>> {
		self.renderer.usage()
	}

	/// The atlases' use since tracking started or the last take, starting over.
	pub fn take_texture_usage(&mut self) -> Option<Vec<bb_render::AtlasUse>> {
		self.renderer.take_usage()
	}

	/// What the renderer and the UI page hold on the GPU.
	pub fn renderer_memory(&self) -> bb_render::RendererMemory {
		let mut memory = self.renderer.memory();
		memory.ui_page = self.front.layer_bytes();
		memory
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

	/// A key went down or up: to the UI, and to the states unless the UI keeps it.
	/// The states get it in the next frame, and a held key every frame until it comes
	/// up (the engine's key queue, as the original's), so held-key scrolling runs at
	/// the frame rate rather than the system's key repeat.
	pub fn key(&mut self, key: &Key) -> Result<(), String> {
		if !self.front.key(key, self.modifier) {
			return Ok(());
		}
		let Some(code) = key.engine else { return Ok(()) };
		self.engine().queue_key(code, key.down, self.modifier);
		Ok(())
	}

	/// The window lost the keyboard: the keys held come up.
	pub fn keyboard_lost(&mut self) {
		self.engine().release_keys();
	}

	fn mouse(&mut self, input: MouseInput) -> Result<(), String> {
		self.front.park();
		self.engine().root_state().mouse_input(self.engine().vm(), input).map(|_| ())
	}

	/// Draw what the player sees now into `target`.
	/// Returns what the map drew (and the cursor): how many sprites, and how many waited
	/// for their atlas.
	pub fn draw(&mut self, gpu: &Gpu, target: &RenderTarget) -> FrameStats {
		self.renderer.begin_frame();
		self.follow_scenes();
		let engine = self.front.engine();
		self.renderer.set_clock(Clock {
			real: engine.time().real_seconds() as f32,
			virtual_time: engine.time().virtual_seconds() as f32,
		});
		self.renderer.set_depth_of_field(engine.video().mode().depth_of_field);
		let (width, height) = (target.width, target.height);
		// The screen the scripts think in maps onto the frame.
		let fit = width as f32 / self.screen().0 as f32;
		let (tactical, world) = (tactical_visible(engine), world_visible(engine));
		let stats = if tactical {
			let scene = engine.tactical_scene();
			let view = WorldView {
				center: [scene.camera.pos.0 as f32, scene.camera.pos.1 as f32],
				zoom: fit / scene.camera.zoom as f32,
				width,
				height,
			};
			engine.sprites().with(|registry| {
				self.renderer.draw_tactical(gpu, target, &scene, registry, &view, Atlases::DrawReady)
			})
		} else if world {
			if let Some(patch) = engine.world_grid().take_fog_patch() {
				self.renderer.update_fog(gpu, &patch);
			}
			let camera = engine.world_camera();
			let view = WorldView {
				center: [camera.pos.0, camera.pos.1],
				zoom: fit / camera.zoom,
				width,
				height,
			};
			// Only the tiles the screen can show: the map is tens of thousands.
			let scene = engine.world_scene_where(|pos| view.shows(pos));
			engine.sprites().with(|registry| {
				self.renderer.draw(gpu, target, &scene, registry, &view, Atlases::DrawReady)
			})
		} else {
			// Neither shows (the main menu, after `World.setVisible(false)`): the UI is
			// drawn over an empty frame, not the last campaign's map.
			target.clear(gpu);
			FrameStats::default()
		};
		// The UI over everything, with the portraits it asked for.
		let renderer = &mut self.renderer;
		self.front
			.serve_procedural(|engine, path| procedural_png(renderer, gpu, engine, path));
		self.front.draw(gpu, target);
		let cursor = self.draw_cursor(gpu, target, fit);
		FrameStats {
			quads: stats.quads + cursor.quads,
			waiting: stats.waiting + cursor.waiting,
			failed: stats.failed + cursor.failed,
		}
	}

	/// What the scenes ask of the renderer and the page this frame: the textures of
	/// those the game discarded go, and what the page kept for them (see
	/// `bb_ui::host::UiHost::release_memory`), and the atlases hinted are warmed (see
	/// `bb_render::residency`). A battle sprite's only while a battle map is set up; the
	/// rest wait for one.
	fn follow_scenes(&mut self) {
		let discards = self.front.engine().take_discards();
		if !discards.is_empty() {
			self.front.discarded();
		}
		let engine = self.front.engine();
		// Fonts mods registered (`ModernEngine.registerFont`), for text drawn from now on.
		for font in engine.font_registrations(self.fonts) {
			self.fonts += 1;
			// Those only for the UI page's fonts are the page's (bb-ui's session).
			if let Some(registration) = bb_render::fonts::Registration::of(&font)
				&& let Err(e) = self.renderer.register_font(registration)
			{
				eprintln!("font {}: {e}", font.path);
			}
		}
		for discard in discards {
			match discard {
				squirrel_bb::Discard::Battle => self.renderer.discard(Discard::Battle),
				squirrel_bb::Discard::Campaign => {
					self.renderer.discard(Discard::Campaign);
					self.battle_atlases.clear();
				}
				// The new campaign draws the old one's atlases again.
				squirrel_bb::Discard::Load => {}
			}
		}
		let sprites = engine.sprites();
		let renderer = &mut self.renderer;
		sprites.with(|registry| renderer.prefetch(registry, sprites.take_hints()));
		let battle_hints = sprites.take_battle_hints();
		let battle_map = engine.has_battle_map();
		self.battle_atlases.extend(battle_hints.iter().cloned());
		if battle_map && !self.battle_map {
			sprites.with(|registry| renderer.prefetch(registry, self.battle_atlases.iter().cloned()));
		} else if battle_map {
			sprites.with(|registry| renderer.prefetch(registry, battle_hints));
		}
		self.battle_map = battle_map;
	}

	/// The mouse cursor the scripts chose, last, over the UI, where they last put it,
	/// unscaled (the original's software cursor); none while the system's shows.
	fn draw_cursor(&mut self, gpu: &Gpu, target: &RenderTarget, fit: f32) -> FrameStats {
		let engine = self.front.engine();
		let (look, (x, y)) = engine.cursor();
		let Some(brush) = look.brush.filter(|_| !look.hardware) else { return FrameStats::default() };
		let since = engine.time().virtual_seconds() as f32 - look.since;
		// Screen pixels from the window's centre, y up.
		let at = [x as f32 * fit - target.width as f32 * 0.5, target.height as f32 * 0.5 - y as f32 * fit];
		engine.sprites().with(|registry| self.renderer.draw_cursor(gpu, target, registry, &brush, since, at))
	}

	/// Whether the scripts want the system's cursor (`UseHardwareCursor`) rather than
	/// the drawn one, and the cursor's name (its `cursors/<name>.cur`).
	pub fn cursor_choice(&self) -> (bool, String) {
		let (look, _) = self.engine().cursor();
		(look.hardware, look.name)
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

/// `World.isVisible()`: the world map is showing (it isn't at the main menu).
fn world_visible(engine: &GameEngine) -> bool {
	let vm = engine.vm();
	SqTable::root(vm)
		.get::<SqTable>("World")
		.and_then(|t| t.get::<SqStrongRef>("isVisible"))
		.and_then(|f| f.call_ret::<bool, _>(vm, ()).ok().flatten())
		.unwrap_or(true)
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
