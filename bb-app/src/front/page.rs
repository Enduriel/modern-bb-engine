//! The scripts with their real UI page, in the browser host (see [`super`]).

use bb_engine::harness::GameEngine;
use bb_render::{Gpu, RenderTarget, UiLayer, UiPage};
use bb_ui::host::{Echo, HostConfig, MouseButton, PageFrames, UiSession};

use crate::game::{Boot, Start};

/// How often the page makes frames a second while a session waits on it without game
/// frames (otherwise one per game frame, see [`Front::draw`]).
const PAGE_WAITING_RATE: i32 = 60;

pub struct Front {
	session: UiSession,
	layer: UiLayer,
	/// Buttons whose press went to the page (bit `1 << id`): their release follows.
	captured: u32,
	/// Whether the last game frame was drawn, and so began the page's frame.
	drawn: bool,
	/// The frame rate for the FPS display, shown at the next step (see [`Front::set_fps`]).
	fps: Option<u32>,
	profiling: Option<crate::script_profile::ScriptProfiling>,
	/// Procedural images (portraits) whose atlases were still loading.
	loading_procedural: Vec<bb_ui::host::ProceduralRequest>,
}

impl Front {
	/// Start the page and the scripts as `boot` says: to the main menu, or on from it into
	/// a new campaign as its Start button does. The campaign's opening event shows once
	/// the world is up.
	pub fn boot(gpu: &Gpu, boot: &Boot) -> Result<Front, String> {
		let (mode, devtools) = (boot.mode, boot.devtools);
		// The page is painted on the GPU, and its paints copied into this texture.
		let page = UiPage::new(gpu)?;
		let config = HostConfig {
			width: mode.width.max(1) as u32,
			height: mode.height.max(1) as u32,
			devtools_port: devtools,
			// The page keeps up with the game's frames, at any refresh rate.
			frames: PageFrames::Game { waiting_rate: PAGE_WAITING_RATE },
			// A live clock behind Time.getExactTime, as the original's: the AI's
			// background work then spreads over frames.
			exact_clock: squirrel_bb::ExactClock::Live,
			// The battle camera glides, as the original's; the AI waits for it.
			battle_camera: squirrel_bb::BattleCamera::Glides,
			// Units hop over time, as the original's; the scripts wait for them.
			battle_motion: squirrel_bb::BattleMotion::Timed,
			mods: boot.mods,
			steam_build: boot.store.steam_build,
			cef_log: boot.cef_log.clone(),
			// Copy and paste reach the player's other programs, as the original's do.
			clipboard: std::sync::Arc::new(match &boot.display {
				Some(display) => crate::clipboard::SystemClipboard::for_window(display),
				None => crate::clipboard::SystemClipboard::new(),
			}),
			sink: Some(super::gpu_page::sink(page.clone())),
			..HostConfig::default()
		};
		// The page opens and loads while the scripts' preload runs (`boot_menu`).
		let session = UiSession::open(boot.data_dir, config, Echo::Problems).map_err(crate::requirements::page_error)?;
		// Before the scripts boot to the menu.
		boot.store.attach(session.engine());
		// The page's links (`openURL`: mods' pages, the version label) open in the
		// system's browser, as the original's do.
		session.protocol().set_url_opener(crate::platform::open_url);
		session.engine().video().set_mode(mode);
		if let Some(settings) = &boot.settings {
			session.engine().apply_game_settings(settings)?;
		}
		// Before the boot: mods read their storages there as they boot (MSU's settings).
		if let Some(dir) = &boot.saves {
			session.engine().saves().set_directory(dir.clone());
		}
		if let Some(port) = devtools {
			super::devtools::announce(port);
		}
		// Portraits are composed on the game thread (`Game::draw`).
		session.queue_procedural(true);
		if boot.sound {
			super::play_on_device(session.engine(), boot.data_dir, boot.mods);
		}
		session.boot_menu().map_err(crate::requirements::page_error)?;
		// The boot is behind us as a scene discarded is (`discarded`): the page gives
		// back what running every screen's and mod's setup left (-45 MB at the main menu),
		// and the allocator its free pages. `BB_BOOT_RELEASE=0` keeps them, to compare.
		if std::env::var_os("BB_BOOT_RELEASE").is_none_or(|v| v != "0") {
			session.host().release_memory();
			crate::platform::trim_allocator();
		}
		if let Some((cols, rows)) = boot.world_size {
			bb_engine::harness::set_world_size(session.engine(), cols, rows)?;
		}
		if let Start::NewCampaign(seed) = &boot.start {
			session.start_campaign(seed)?;
		}
		let archives = crate::game::archives(boot.data_dir, boot.mods).map_err(|e| e.to_string())?;
		Ok(Front {
			session,
			layer: UiLayer::new(gpu, page),
			captured: 0,
			drawn: false,
			fps: None,
			profiling: crate::script_profile::ScriptProfiling::from_env(&archives),
			loading_procedural: Vec::new(),
		})
	}

	pub fn engine(&self) -> &GameEngine {
		self.session.engine()
	}

	/// When the browser wants its work run next, if it asked: on macOS, where the game's
	/// thread runs it ([`pump`](Self::pump)); never elsewhere.
	pub fn pump_due(&self) -> Option<std::time::Instant> {
		self.session.host().pump_due()
	}

	/// Run the browser's work if it is due (macOS).
	pub fn pump(&self) {
		self.session.host().pump()
	}

	/// Have `waker` wake the window's loop when the browser wants its work run sooner.
	pub fn set_pump_waker(&self, waker: Option<super::Waker>) {
		self.session.host().set_pump_waker(waker);
	}

	/// The page and the game together, for checks.
	pub fn session(&self) -> &UiSession {
		&self.session
	}

	/// Advance the game by `ms` of real time, answering the page. The page then runs
	/// on while the frame is drawn, until the scripts next run.
	pub fn step(&mut self, ms: f64) -> Result<(), String> {
		// The FPS display's new value, with the page parked, as the scripts' calls are.
		if let Some(fps) = self.fps.take() {
			let park = bb_render::frame_timing::span("page park");
			self.session.park();
			drop(park);
			self.session.set_fps(&fps.to_string());
		}
		let session = &self.session;
		match &mut self.profiling {
			Some(profiling) => profiling.step(session.engine().vm(), || session.step_unparked(ms))?,
			None => session.step_unparked(ms)?,
		}
		// A page frame for each game frame: after its draw (`draw`), or now for a frame
		// that wasn't drawn (a loop that only steps), so the page never stands still.
		if !std::mem::replace(&mut self.drawn, false) {
			let _begin = bb_render::frame_timing::span("page begin frame");
			self.session.host().begin_frame();
		}
		Ok(())
	}

	/// Before the scripts run outside a step (the player's input): the page waits
	/// until the next step lets it go on, as the original's UI runs between the
	/// game's scripts.
	pub fn park(&mut self) {
		self.session.park();
	}

	/// Run the JS `code` in the page, parked as for the scripts' calls into it.
	pub fn run_page_script(&mut self, code: &str) -> Result<(), String> {
		self.session.park();
		self.session.eval(code).map(|_| ())
	}

	/// Show the FPS display (vanilla's FPS module on the root screen), or hide it. Its
	/// value starts empty until the first [`set_fps`](Self::set_fps).
	pub fn show_fps(&mut self, shown: bool) -> Result<(), String> {
		self.fps = None;
		self.session.park();
		self.session.show_fps_module(shown)?;
		if shown {
			self.session.set_fps("");
		}
		Ok(())
	}

	/// Show `fps` on the FPS display, at the next step (with the page's other changes).
	pub fn set_fps(&mut self, fps: u32) {
		self.fps = Some(fps);
	}

	/// The game discarded a scene: the page gives back what it kept for it.
	pub fn discarded(&mut self) {
		self.session.host().release_memory();
	}

	/// Paint the page at `scale` frame pixels per pixel of the game's screen (the
	/// renderer's `fit`), from the next step on: sharp on a Retina display or a screen
	/// scaled to its window.
	pub fn set_frame_scale(&mut self, scale: f64) {
		self.session.set_frame_scale(scale);
	}

	/// The cursor moved to the screen pixel `(x, y)`: the page's mouse follows, and
	/// whether it's over the UI is what the page shows there.
	pub fn cursor(&mut self, x: i64, y: i64) {
		let over_ui = self.session.is_over_ui(x, y);
		self.session.engine().ui().set_cursor(x, y, over_ui);
		let (px, py) = self.session.to_page(x, y);
		self.session.host().mouse_move(px, py);
	}

	/// Whether the page takes button `id`'s press (over the UI) or release (after its
	/// press went to the page).
	pub fn button(&mut self, x: i64, y: i64, id: i64, down: bool) -> bool {
		let button = match id {
			1 => MouseButton::Left,
			2 => MouseButton::Right,
			3 => MouseButton::Middle,
			_ => return false,
		};
		let bit = 1 << id;
		let to_page = if down {
			self.session.is_over_ui(x, y)
		} else {
			self.captured & bit != 0
		};
		if !to_page {
			return false;
		}
		if down {
			self.captured |= bit;
		} else {
			self.captured &= !bit;
		}
		let (px, py) = self.session.to_page(x, y);
		self.session.host().mouse_button(px, py, button, down);
		true
	}

	/// Whether the page takes a turn of the wheel (over the UI).
	pub fn wheel(&mut self, x: i64, y: i64, up: bool) -> bool {
		if !self.session.is_over_ui(x, y) {
			return false;
		}
		let (px, py) = self.session.to_page(x, y);
		self.session.host().wheel(px, py, if up { 1 } else { -1 });
		true
	}

	/// The modifier keys held now (the `KeyModifier` mask): the page's mouse events
	/// carry them.
	pub fn set_modifier(&mut self, modifier: i64) {
		self.session.host().set_modifiers(bb_ui::host::modifiers_of_mask(modifier));
	}

	/// Answer the procedural images (portraits) the page asked for with `compose`, given
	/// the engine and the path after `procedural/`: those still loading are asked again
	/// at the next call.
	pub fn serve_procedural(&mut self, mut compose: impl FnMut(&GameEngine, &str) -> crate::procedural::Procedural) {
		let mut requests = std::mem::take(&mut self.loading_procedural);
		requests.extend(self.session.take_procedural());
		for request in requests {
			match compose(self.session.engine(), &request.path) {
				crate::procedural::Procedural::Image(png) => request.answer(Some(png)),
				crate::procedural::Procedural::Fail => request.fail(),
				crate::procedural::Procedural::Loading => self.loading_procedural.push(request),
			}
		}
	}

	/// What copying the page's paints (and reading them back) has cost so far.
	pub fn copy_stats(&self) -> bb_render::UiCopyStats {
		self.layer.page().copy_stats()
	}

	/// The bytes the page's texture takes on the GPU.
	pub fn layer_bytes(&self) -> u64 {
		self.layer.bytes()
	}

	/// How many game frames so far the page rested through (no turn, no frame begun).
	pub fn page_rested(&self) -> u64 {
		self.session.host().rested_frames()
	}

	/// How many times the page has painted: each paint is a new picture of it.
	pub fn paints(&self) -> u64 {
		self.session.host().paints()
	}

	/// Draw the page's latest paint over `target` (each paint is copied into the layer's
	/// texture as it lands).
	pub fn draw(&mut self, gpu: &Gpu, target: &RenderTarget) {
		use bb_render::frame_timing::span;
		let draw = span("ui layer draw");
		self.layer.draw(gpu, target);
		drop(draw);
		// The page's next frame, painted by the next game frame's draw (bb-ui's
		// `PageFrames::Game`).
		let _begin = span("page begin frame");
		self.session.host().begin_frame();
		self.drawn = true;
	}
}

