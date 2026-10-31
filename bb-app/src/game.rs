//! The game behind the window: the engine, booted to the main menu or into a new
//! campaign, stepped by the frame's real time, fed the player's input, and drawn by the
//! renderer into a frame the window presents. The scripts' UI is the [`Front`]'s: headless, or with the `ui`
//! feature the real page drawn over the frame.

use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use bb_engine::files::GameFiles;
use bb_assets::CursorImage;
use bb_engine::harness::{self, GameEngine};
use bb_render::world::WorldView;
use bb_render::{Atlases, Clock, Discard, FrameStats, Gpu, RenderTarget, WorldMap, WorldRenderer};
use squirrel::{SqStrongRef, SqTable};
use squirrel_bb::{MouseInput, VideoMode, VideoSettings};

use crate::fps::FpsMeter;
use crate::front::{Front, Key};
use crate::game_cursor::{self, CursorPictures, CursorShow};
use crate::shown::{self, CursorLook, Look, Shown, Under};
use crate::world_scale::WorldScale;

/// The longest real time one frame advances the game by.
const MAX_FRAME_MS: f64 = 250.0;

pub struct Game {
	front: Front,
	renderer: WorldRenderer,
	/// The battle sprites' atlases hinted while no battle map was set up (the company's
	/// units on the world map), warmed whenever one is; and whether one was last frame.
	battle_atlases: HashSet<String>,
	battle_map: bool,
	/// The world map's revision when the campaign was last left: until it changes (a
	/// campaign made or loaded), the map's tiles are the old campaign's, and the water's
	/// and the fog's textures aren't wanted.
	left_map: Option<u64>,
	/// How many of the fonts mods registered the renderer has.
	fonts: usize,
	cursor: (i64, i64),
	/// The real time of the frames whose step didn't run (the page was too busy to
	/// park), for the next step that does.
	unstepped_ms: f64,
	/// The pointer moved since its last move was delivered (see
	/// [`mouse_moved`](Self::mouse_moved)).
	moved: bool,
	/// The game's cursor shown as the system's where it can be (see [`game_cursor`]),
	/// rather than drawn; and its pictures.
	system_cursor: bool,
	cursor_pictures: CursorPictures,
	/// Frame pixels per pixel of the drawn cursor's picture (see
	/// [`set_cursor_scale`](Self::set_cursor_scale)).
	cursor_scale: f32,
	/// The modifier keys held (`KeyModifier` mask), sent with every event.
	modifier: i64,
	/// What the last frame drawn showed (see [`Game::draw_if_changed`]).
	shown: Shown,
	/// Keys pressed while the update notice showed, until released (see
	/// [`Game::notice_keys`]).
	notice_held: HashSet<i64>,
	/// While the FPS display shows, what measures the frame rate for it.
	fps: Option<FpsMeter>,
	/// Battles are drawn in steps (see `squirrel_bb::scene::steps`); `BB_SCENE_STEPS=0`
	/// draws them at the clocks as they are.
	scene_steps: bool,
	/// The world drawn at the screen's size and enlarged, on a Retina Mac (see
	/// [`crate::world_scale`]).
	world_scale: WorldScale,
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
	/// The gameplay, control and sound settings (the settings file's); the harness's
	/// without (as for tests).
	pub settings: Option<squirrel_bb::GameSettings>,
	/// Serve Chrome DevTools for the page at this port (the page's front only).
	pub devtools: Option<u16>,
	/// Where the saves (and mods' storages, which they read as they boot) are kept as
	/// files; in memory without one.
	pub saves: Option<std::path::PathBuf>,
	/// Where CEF writes its log (the page's front only): beside log.html; without one, in
	/// its temporary profile, gone when it closes.
	pub cef_log: Option<std::path::PathBuf>,
	/// The store the install is from, and the achievements' service.
	pub store: Store,
	/// The update check, with the page's front: bb's asks GitHub; without one (tests)
	/// none is made.
	pub update_check: Option<crate::update_check::UpdateCheck>,
	/// The game window's display, whose clipboard the page's copy and paste use (on
	/// Wayland, served through the window's connection); the system's clipboard
	/// programs without one (the page's front only).
	pub display: Option<winit::event_loop::OwnedDisplayHandle>,
}

/// The store the install is from (the scripts' `isSteamBuild()`), and the service the
/// achievement and stat natives call (Steam's, see [`crate::steam`]).
#[derive(Clone)]
pub struct Store {
	pub steam_build: bool,
	/// `None`: the engine's own, which unlocks nothing ([`squirrel_bb::OfflineStats`]).
	pub user_stats: Option<std::rc::Rc<dyn squirrel_bb::UserStats>>,
}

impl Default for Store {
	/// Steam's build with no achievements, as the harness's engines are (for tests).
	fn default() -> Self {
		Store { steam_build: true, user_stats: None }
	}
}

impl std::fmt::Debug for Store {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Store")
			.field("steam_build", &self.steam_build)
			.field("user_stats", &self.user_stats.as_ref().map(|_| "set"))
			.finish()
	}
}

impl Store {
	/// Give the engine the service, before its scripts boot.
	pub fn attach(&self, engine: &GameEngine) {
		if let Some(stats) = &self.user_stats {
			engine.user_stats().set(stats.clone());
		}
	}
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
			settings: None,
			devtools,
			saves: None,
			cef_log: None,
			store: Store::default(),
			update_check: None,
			display: None,
		};
		Game::start(gpu, &boot)
	}

	/// Boot the engine as `boot` says: to the main menu, or on into a new campaign.
	pub fn start(gpu: &Gpu, boot: &Boot) -> Result<Game, String> {
		let io = |e: std::io::Error| e.to_string();
		let archives = archives(boot.data_dir, boot.mods).map_err(io)?;
		let files = Arc::new(GameFiles::open(archives.clone()).map_err(io)?);
		let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
		let mut renderer = WorldRenderer::new(gpu, files.clone(), threads);
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
			left_map: None,
			cursor: (0, 0),
			unstepped_ms: 0.0,
			moved: false,
			system_cursor: false,
			cursor_pictures: CursorPictures::new(files),
			cursor_scale: 1.0,
			modifier: 0,
			shown: Shown::default(),
			notice_held: HashSet::new(),
			fps: None,
			scene_steps: std::env::var_os("BB_SCENE_STEPS").is_none_or(|v| v != "0"),
			world_scale: WorldScale::new(false),
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

	/// Ask for the engine's latest release again, as at the start (only if the boot
	/// asked for the update check, and only with the page's front).
	pub fn check_for_update(&mut self) {
		self.front.check_for_update();
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

	/// Run the JS `code` in the UI page (`--at-page`), as the scripts' calls into it run.
	pub fn run_page_script(&mut self, code: &str) -> Result<(), String> {
		self.front.run_page_script(code)
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

	/// When the renderer last drew each texture and page, and its glyph pages (for the
	/// memory snapshot).
	pub fn renderer_residency(&self) -> bb_render::ResidencyDetail {
		self.renderer.residency_detail()
	}

	/// The world camera's zoom (for the frame times' log).
	pub fn world_zoom(&self) -> f32 {
		self.front.engine().world_camera().zoom
	}

	/// The engine's real time, in seconds: the clock scripts read (`Time.getRealTimeF`).
	pub fn real_seconds(&self) -> f64 {
		self.front.engine().time().real_seconds()
	}

	/// Advance the game by `ms` of real time: a step, as the FPS display counts them
	/// beside the frames presented ([`Game::presented`]).
	pub fn step(&mut self, ms: f64) -> Result<(), String> {
		let _span = bb_render::frame_timing::span("step");
		if let Some(fps) = self.fps.as_mut().and_then(|meter| meter.frame(std::time::Instant::now())) {
			self.front.set_fps(fps);
		}
		self.queue_move();
		// The real time of steps that waited on the page goes to the next that runs.
		let ms = ms + std::mem::take(&mut self.unstepped_ms);
		if !self.front.step(ms.clamp(0.0, MAX_FRAME_MS))? {
			self.unstepped_ms = ms;
		}
		Ok(())
	}

	/// The pointer's last move since the last frame, to the page now and to the states
	/// in this frame, after its keys (see [`mouse_moved`](Self::mouse_moved)).
	fn queue_move(&mut self) {
		if let Some(input) = self.take_move() {
			self.engine().queue_mouse(input);
		}
	}

	/// The pointer's last move since the last frame, delivered now, before a button or
	/// the wheel: they come after the move to where they happened, as the original's
	/// queue keeps its events in order.
	fn deliver_move(&mut self) -> Result<(), String> {
		match self.take_move() {
			Some(input) => self.mouse(input),
			None => Ok(()),
		}
	}

	/// The pointer's move not yet delivered, if it moved: handed to the page now, and
	/// given back for the states.
	fn take_move(&mut self) -> Option<MouseInput> {
		if !std::mem::take(&mut self.moved) {
			return None;
		}
		bb_render::frame_timing::count("mouse moves delivered", 1);
		let (x, y) = self.cursor;
		let page = bb_render::frame_timing::span("input page");
		self.front.cursor(x, y);
		drop(page);
		Some(MouseInput { id: 6, state: 2, x, y, modifier: self.modifier })
	}

	/// Show the FPS display (vanilla's FPS module, which the original never shows), or
	/// hide it. It is off until shown, and the page vanilla's.
	pub fn show_fps(&mut self, shown: bool) -> Result<(), String> {
		self.front.show_fps(shown)?;
		self.fps = shown.then(FpsMeter::default);
		Ok(())
	}

	/// Whether the FPS display shows.
	pub fn fps_shown(&self) -> bool {
		self.fps.is_some()
	}

	/// The last step's frame was presented on the window: the FPS display counts it.
	pub fn presented(&mut self) {
		if let Some(fps) = &mut self.fps {
			fps.presented();
		}
	}

	/// The cursor moved to `(x, y)` in the game's screen pixels. The moves since the
	/// last frame are merged: the page and the states get the last one as the next frame
	/// starts (the states after its keys, see `Engine::queue_mouse`), or before a button
	/// or the wheel. Every pointer move is `MouseState.Moved`, button held or not
	/// (disassembled). The original's states get a few moves a frame, not one a system
	/// event: its system merges them (probed 2026-10-23: 1 to 4 a frame at 60 fps from a
	/// thousand moves a second, consecutive positions), and its input manager hands each
	/// on in order at the frame (`0x5b6220`, `0x551550`).
	pub fn mouse_moved(&mut self, x: i64, y: i64) -> Result<(), String> {
		bb_render::frame_timing::count("mouse moves", 1);
		self.cursor = (x, y);
		self.moved = true;
		Ok(())
	}

	/// Button `id` (1 left, 2 right, 3 middle, 4 back, 5 forward) went down or up: to
	/// the UI, or to the states.
	pub fn mouse_button(&mut self, id: i64, down: bool) -> Result<(), String> {
		self.deliver_move()?;
		let (x, y) = self.cursor;
		let page = bb_render::frame_timing::span("input page");
		let to_page = self.front.button(x, y, id, down);
		drop(page);
		if to_page {
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
		self.deliver_move()?;
		let (x, y) = self.cursor;
		let page = bb_render::frame_timing::span("input page");
		let to_page = self.front.wheel(x, y, up);
		drop(page);
		if to_page {
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

	/// The window got the keyboard or lost it: the engine's key queue forgets its keys,
	/// releasing none, and drops the keys that come meanwhile, as the original's main
	/// loop does (see `squirrel_bb::key_queue`).
	pub fn keyboard_focus(&mut self, focused: bool) {
		self.engine().keyboard_focus(focused);
	}

	/// The modifier keys held now (see `keys::modifier`).
	pub fn set_modifier(&mut self, modifier: i64) {
		self.modifier = modifier;
		self.front.set_modifier(modifier);
	}

	/// A key went down or up, or repeats: to the engine's key queue, as the original's.
	/// From the next frame the UI gets it and then the states, whatever has the page's
	/// focus (the original's UI never keeps a key from them: its listener `0x56cb40`
	/// always answers "not handled"; so a `c` typed into a name field also toggles the
	/// character screen, as MSU's `blockSQInput` setting describes). A held key comes
	/// to both every frame until it's released, so held-key scrolling runs at the frame
	/// rate, not the system's key repeat. What the key typed goes to the UI after it,
	/// each repeat again; Tab types nothing there, nor does a key with Alt held (Windows
	/// makes that a system character, which SFML doesn't take; probed 2026-10-21).
	/// AltGr, which Windows reports as Ctrl+Alt, types. Nor does a key with the Mac's
	/// Command held (a shortcut there, as Cmd+C; `KEY_MODIFIER_COMMAND`).
	pub fn key(&mut self, key: &Key) -> Result<(), String> {
		if self.notice_keys(key) {
			return Ok(());
		}
		if let Some(code) = key.engine {
			self.engine().queue_key(code, key.down, self.modifier);
		}
		let alt_only = self.modifier & 4 != 0 && self.modifier & 2 == 0;
		let command = self.modifier & squirrel_bb::KEY_MODIFIER_COMMAND != 0;
		if key.down && !alt_only && !command {
			for c in key.text.iter().flat_map(|t| t.chars()).filter(|&c| c != '\t') {
				self.engine().queue_text(c, self.modifier);
			}
		}
		Ok(())
	}

	/// Whether the update notice takes `key`: every key while it shows (none reaches the
	/// states or the page), Escape choosing Continue and Enter Open link as they go down;
	/// and a key pressed while it showed until its release, though the notice has gone,
	/// so Escape's release doesn't reach the main menu (where it goes back, or quits).
	fn notice_keys(&mut self, key: &Key) -> bool {
		let code = key.engine;
		if self.front.update_notice_shown() {
			if let Some(code) = code.filter(|_| key.down) {
				self.notice_held.insert(code);
				let choice = match code {
					crate::keys::ESCAPE => Some("continue"),
					crate::keys::ENTER => Some("open"),
					_ => None,
				};
				if let Some(choice) = choice.filter(|_| !key.repeat) {
					self.front.choose_in_update_notice(choice);
				}
			}
			if let Some(code) = code.filter(|_| !key.down) {
				self.notice_held.remove(&code);
			}
			return true;
		}
		match code {
			Some(code) if self.notice_held.contains(&code) => {
				if !key.down {
					self.notice_held.remove(&code);
				}
				true
			}
			_ => false,
		}
	}

	/// `input` to the states now, between frames: the page parked first. A page too
	/// busy to park holds it until the next frame that runs, after the moves and
	/// buttons queued before it (see `Engine::queue_mouse`).
	fn mouse(&mut self, input: MouseInput) -> Result<(), String> {
		let park = bb_render::frame_timing::span("input park");
		let parked = self.front.park();
		drop(park);
		if !parked {
			self.engine().queue_mouse(input);
			return Ok(());
		}
		let _scripts = bb_render::frame_timing::span("input scripts");
		self.engine().root_state().mouse_input(self.engine().vm(), input).map(|_| ())
	}

	/// Draw what the player sees now into `target`.
	/// Returns what the map drew (and the cursor): how many sprites, and how many waited
	/// for their atlas.
	pub fn draw(&mut self, gpu: &Gpu, target: &RenderTarget) -> FrameStats {
		self.draw_unless_shown(gpu, target, false).unwrap_or_default()
	}

	/// As [`draw`](Self::draw), but when the frame would show just what the last one
	/// drawn showed (see [`crate::shown`]), draw nothing and return `None`: `target`
	/// still holds that frame, and the window can leave it up.
	pub fn draw_if_changed(&mut self, gpu: &Gpu, target: &RenderTarget) -> Option<FrameStats> {
		self.draw_unless_shown(gpu, target, true)
	}

	fn draw_unless_shown(&mut self, gpu: &Gpu, target: &RenderTarget, compare: bool) -> Option<FrameStats> {
		let follow = bb_render::frame_timing::span("follow scenes");
		self.follow_scenes();
		drop(follow);
		// The portraits the UI asked for, drawn or not: the page paints them.
		let procedural = bb_render::frame_timing::span("portraits");
		let renderer = &mut self.renderer;
		self.front
			.serve_procedural(|engine, path| crate::procedural::answer(renderer, gpu, engine, path, Atlases::Ask));
		drop(procedural);
		let drawn_cursor = self.cursor_show() == CursorShow::Drawn;
		let (width, height) = (target.width, target.height);
		// The screen the scripts think in maps onto the frame. The page is painted at the
		// frame's pixels (see `Front::set_frame_scale`).
		let fit = width as f32 / self.screen().0 as f32;
		self.front.set_frame_scale(f64::from(fit));
		// The world and battles: at the frame's size, or at the screen's and enlarged
		// (`world_scale`).
		let enlarge = self.world_scale.factor((width, height), self.screen());
		let (world_fit, world_width, world_height) = match enlarge {
			Some(n) => (1.0, width / n, height / n),
			None => (fit, width, height),
		};
		let engine = self.front.engine();
		let (tactical, world) = (tactical_visible(engine), world_visible(engine));
		// A campaign's map, made or loaded: its water and fog load before it shows (the
		// loading screen), not with its first frame.
		let grid = engine.world_grid();
		let (map_width, map_height) = grid.extent();
		if !tactical && map_width > 0.0 && map_height > 0.0 && self.left_map != Some(grid.map_revision()) {
			self.renderer.prefetch_world(gpu, grid.fog_of_war());
		}
		let virtual_time = engine.time().virtual_seconds();
		let real_time = engine.time().real_seconds() as f32;
		// A world map whose fog of war just changed looks different every frame.
		let mut comparable = true;
		let mut detail_changes = Vec::new();
		let mut map_edits = Vec::new();
		let depth_of_field = engine.video().mode().depth_of_field;
		// A battle's animations move in steps (see `squirrel_bb::scene::steps`): of a
		// pixel of this frame, from the next frame on.
		engine.set_scene_steps(self.scene_steps.then_some(world_fit));
		let under = if tactical {
			// What the battle draws is all in its scene (see `crate::shown`).
			let scene_span = bb_render::frame_timing::span("battle scene");
			let scene = engine.tactical_scene();
			drop(scene_span);
			Under::Battle {
				view: ([scene.camera.pos.0 as f32, scene.camera.pos.1 as f32], world_fit / scene.camera.zoom as f32),
				scene: Box::new(scene),
				fit: world_fit,
				depth_of_field,
			}
		} else if world {
			if let Some(patch) = engine.world_grid().take_fog_patch() {
				self.renderer.update_fog(gpu, &patch);
				comparable = false;
			}
			let camera = engine.world_camera();
			let view = WorldView {
				center: [camera.pos.0, camera.pos.1],
				zoom: world_fit / camera.zoom,
				fit: world_fit,
				width: world_width,
				height: world_height,
			};
			// The map's terrain and details are kept by the renderer (built again when
			// the map's generation changes, edited as details are spawned and removed,
			// the details changed redrawn); the scene is the rest, without its tiles,
			// built only to be drawn: its key tells whether it changed.
			let key = engine.world_scene_key();
			let grid = engine.world_grid();
			let changes = grid.take_detail_changes();
			let edits = grid.take_map_edits();
			// A detail spawned, removed or whose look changed draws (the renderer takes
			// them once).
			comparable &= changes.is_empty() && edits.is_empty();
			// Without a map (the main menu before a campaign) the water and the fog draw
			// nothing, and so don't drift.
			let map = key.extent.0 > 0.0 && key.extent.1 > 0.0;
			// The fog drifts with real time, in steps (see `bb_render::fog_drift_time`).
			let fog = (map && key.fog_of_war)
				.then(|| shown::view_rect(view.center, view.zoom, world_width, world_height))
				.filter(|rect| !grid.fog_clear_within(*rect))
				.map(|_| bb_render::fog_drift_time(real_time, view.zoom));
			detail_changes = changes;
			map_edits = edits;
			Under::World {
				scene: key,
				view: (view.center, view.zoom),
				virtual_time: if map { virtual_time } else { 0.0 },
				fog,
				map_revision: grid.map_revision(),
			}
		} else {
			Under::Nothing
		};
		let look = Look {
			size: (width, height),
			page_paints: self.front.paints(),
			cursor: if drawn_cursor { self.cursor_look(target, fit) } else { None },
			fonts: self.fonts,
			under,
		};
		let compare_span = bb_render::frame_timing::span("compare");
		let same = compare && comparable && self.shown.same(Some(&look));
		drop(compare_span);
		if same {
			return None;
		}
		self.renderer.begin_frame();
		let engine = self.front.engine();
		self.renderer.set_clock(Clock {
			real: real_time,
			virtual_time: virtual_time as f32,
		});
		self.renderer.set_depth_of_field(depth_of_field);
		// The world's target: the frame, or one of the screen's size enlarged into it.
		let world_target = match enlarge {
			Some(_) if !matches!(look.under, Under::Nothing) => self.world_scale.target(gpu, (world_width, world_height)),
			_ => target,
		};
		let stats = if let Under::Battle { scene, view: (center, zoom), .. } = &look.under {
			let _span = bb_render::frame_timing::span("battle draw");
			let view = WorldView {
				fit: world_fit,
				center: *center,
				zoom: *zoom,
				width: world_width,
				height: world_height,
			};
			engine.sprites().with(|registry| {
				let battle = self.renderer.draw_tactical(gpu, world_target, scene, registry, &view, Atlases::DrawReady);
				// The camera's orientation overlays, over the battle and under the UI.
				let overlays =
					self.renderer.draw_orientation(gpu, world_target, registry, &scene.orientation, world_fit, Atlases::DrawReady);
				FrameStats {
					quads: battle.quads + overlays.quads,
					waiting: battle.waiting + overlays.waiting,
					failed: battle.failed + overlays.failed,
				}
			})
		} else if let Under::World { view: (center, zoom), .. } = &look.under {
			let scene = {
				let _span = bb_render::frame_timing::span("world scene");
				engine.world_scene_where(|_| false)
			};
			let view = WorldView {
				fit: world_fit,
				center: *center,
				zoom: *zoom,
				width: world_width,
				height: world_height,
			};
			let grid = engine.world_grid();
			let tiles = || grid.scene_map();
			let map = WorldMap {
				revision: grid.map_generation(),
				tiles: &tiles,
				edits: &map_edits,
				changes: &detail_changes,
			};
			let _span = bb_render::frame_timing::span("world draw");
			engine.sprites().with(|registry| {
				self.renderer.draw_kept(gpu, world_target, &scene, &map, registry, &view, Atlases::DrawReady)
			})
		} else {
			// Neither shows (the main menu, after `World.setVisible(false)`): the UI is
			// drawn over an empty frame, not the last campaign's map.
			target.clear(gpu);
			FrameStats::default()
		};
		if enlarge.is_some() && !matches!(look.under, Under::Nothing) {
			let _span = bb_render::frame_timing::span("world enlarge");
			self.world_scale.enlarge(gpu, target);
		}
		// The UI over everything.
		let front_span = bb_render::frame_timing::span("ui page");
		self.front.draw(gpu, target);
		drop(front_span);
		let cursor_span = bb_render::frame_timing::span("cursor draw");
		let cursor = if drawn_cursor { self.draw_cursor(gpu, target, fit) } else { FrameStats::default() };
		drop(cursor_span);
		let stats = FrameStats {
			quads: stats.quads + cursor.quads,
			waiting: stats.waiting + cursor.waiting,
			failed: stats.failed + cursor.failed,
		};
		// A frame drawn without what was still loading is drawn again.
		self.shown.drawn((comparable && stats.waiting == 0).then_some(look));
		Some(stats)
	}

	/// A battle shows (`Tactical.isVisible()`).
	pub fn in_battle(&self) -> bool {
		tactical_visible(self.front.engine())
	}

	/// The next frame is drawn, whatever it shows (the window's surface was replaced).
	pub fn forget_shown(&mut self) {
		self.shown.forget();
	}

	/// The cursor as [`draw_cursor`](Self::draw_cursor) draws it, for telling frames
	/// apart.
	fn cursor_look(&self, target: &RenderTarget, fit: f32) -> Option<CursorLook> {
		let engine = self.front.engine();
		let (look, (x, y)) = engine.cursor();
		let brush = look.brush.filter(|_| !look.hardware)?;
		let since = engine.time().virtual_seconds() as f32 - look.since;
		let frame = engine.sprites().with(|registry| {
			let (_, sprite) = registry.get(&brush)?;
			bb_render::cursor_frame_at(sprite, since).map(|f| (f.uv, f.rect))
		});
		Some(CursorLook {
			brush,
			at: cursor_at(target, fit, x, y),
			scale: self.cursor_scale,
			frame,
		})
	}

	/// What the scenes ask of the renderer and the page this frame: the textures of
	/// those the game discarded go, and what the page kept for them (see
	/// `bb_ui::host::UiHost::release_memory`), and the atlases hinted are warmed (see
	/// `bb_render::residency`). A battle sprite's only while a battle map is set up; the
	/// rest wait for one.
	fn follow_scenes(&mut self) {
		let discards = self.front.engine().take_discards();
		let discarded = !discards.is_empty();
		if discarded {
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
		let discard_span = bb_render::frame_timing::span("scene discard");
		for discard in discards {
			match discard {
				squirrel_bb::Discard::Battle => self.renderer.discard(Discard::Battle),
				squirrel_bb::Discard::Campaign => {
					self.renderer.discard(Discard::Campaign);
					self.left_map = Some(engine.world_grid().map_revision());
					self.battle_atlases.clear();
				}
				// The new campaign draws the old one's atlases again.
				squirrel_bb::Discard::Load => {}
			}
		}
		drop(discard_span);
		// What the scene's objects, sounds and the page's browser side freed goes back
		// to the system (see `platform::trim_allocator`).
		if discarded {
			let _span = bb_render::frame_timing::span("trim");
			crate::platform::trim_allocator();
		}
		let _prefetch = bb_render::frame_timing::span("prefetch");
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
		let at = cursor_at(target, fit, x, y);
		let scale = self.cursor_scale;
		engine.sprites().with(|registry| self.renderer.draw_cursor(gpu, target, registry, &brush, since, at, scale))
	}

	/// Draw the game's cursor at `scale` frame pixels per pixel of its picture: 1, or a
	/// Retina display's backing factor, where the game's screen is in points (see
	/// `screen_scale`), so it is the size it is on a display without Retina, as the
	/// system's shows it ([`crate::system_cursor`]).
	pub fn set_cursor_scale(&mut self, scale: f32) {
		self.cursor_scale = scale;
	}

	/// Draw the world and battles at the game's screen's size and enlarge them where the
	/// frame is a whole multiple of it (see [`crate::world_scale`]); off, at the frame's pixels.
	pub fn draw_world_at_screen_size(&mut self, on: bool) {
		self.world_scale.on = on;
	}

	/// Show the game's cursor as the system's where it can be, rather than draw it
	/// (see [`game_cursor`]); it is drawn until this is set, as tests and tools want it.
	pub fn use_system_cursor(&mut self, on: bool) {
		self.system_cursor = on;
	}

	/// What shows as the mouse cursor now (see [`game_cursor`]): drawn by the game, or
	/// which the system shows.
	pub fn cursor_show(&mut self) -> CursorShow {
		let engine = self.front.engine();
		let (look, at) = engine.cursor();
		let at_mouse = at == self.cursor;
		let frame = match &look.brush {
			Some(brush) if self.system_cursor && at_mouse && !look.hardware => {
				let since = engine.time().virtual_seconds() as f32 - look.since;
				let pictures = &mut self.cursor_pictures;
				engine.sprites().with(|registry| {
					let built = pictures.get(registry, brush, || cursor_names(engine))?;
					let (_, sprite) = registry.get(brush)?;
					let frame = bb_render::cursor_frame_index_at(sprite, since);
					(frame < built.len()).then_some(frame)
				})
			}
			_ => None,
		};
		game_cursor::choose(&look, self.system_cursor, at_mouse, frame)
	}

	/// The pictures of the cursor sprite `brush`'s frames for the system, once
	/// [`cursor_show`](Self::cursor_show) has shown it.
	pub fn cursor_pictures(&mut self, brush: &str) -> Option<Rc<[CursorImage]>> {
		let engine = self.front.engine();
		let pictures = &mut self.cursor_pictures;
		engine.sprites().with(|registry| pictures.get(registry, brush, || cursor_names(engine)))
	}
}

/// Where the cursor at the screen pixel `(x, y)` is drawn: frame pixels from the frame's
/// centre, y up.
fn cursor_at(target: &RenderTarget, fit: f32, x: i64, y: i64) -> [f32; 2] {
	[x as f32 * fit - target.width as f32 * 0.5, target.height as f32 * 0.5 - y as f32 * fit]
}

/// The cursor sprites the scripts name (`Const.UI.Cursor`'s values, mods' included).
fn cursor_names(engine: &GameEngine) -> Vec<String> {
	let vm = engine.vm();
	let Some(cursors) = SqTable::root(vm)
		.get::<SqTable>("Const")
		.and_then(|c| c.get::<SqTable>("UI"))
		.and_then(|ui| ui.get::<SqTable>("Cursor"))
	else {
		return Vec::new();
	};
	cursors.keys().iter().filter_map(|key| cursors.get::<String>(key)).collect()
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
