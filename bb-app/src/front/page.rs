//! The scripts with their real UI page, in the browser host (see [`super`]).

use bb_engine::harness::GameEngine;
use bb_render::{Gpu, RenderTarget, UiLayer, UiRegion};
use bb_ui::host::{Echo, HostConfig, Modifiers, MouseButton, PageFrames, UiSession};

use super::Key;
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
}

impl Front {
	/// Start the page and the scripts as `boot` says: to the main menu, or on from it into
	/// a new campaign as its Start button does. The campaign's opening event shows once
	/// the world is up.
	pub fn boot(gpu: &Gpu, boot: &Boot) -> Result<Front, String> {
		let (mode, devtools) = (boot.mode, boot.devtools);
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
			..HostConfig::default()
		};
		let session = UiSession::start(boot.data_dir, config, Echo::Problems)?;
		// Before the scripts boot to the menu.
		boot.store.attach(session.engine());
		// The page's links (`openURL`: mods' pages, the version label) open in the
		// system's browser, as the original's do.
		session.protocol().set_url_opener(crate::platform::open_url);
		session.engine().video().set_mode(mode);
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
		session.boot_menu()?;
		if let Some((cols, rows)) = boot.world_size {
			bb_engine::harness::set_world_size(session.engine(), cols, rows)?;
		}
		if let Start::NewCampaign(seed) = &boot.start {
			session.start_campaign(seed)?;
		}
		Ok(Front {
			session,
			layer: UiLayer::new(gpu),
			captured: 0,
			drawn: false,
		})
	}

	pub fn engine(&self) -> &GameEngine {
		self.session.engine()
	}

	/// The page and the game together, for checks.
	pub fn session(&self) -> &UiSession {
		&self.session
	}

	/// Advance the game by `ms` of real time, answering the page. The page then runs
	/// on while the frame is drawn, until the scripts next run.
	pub fn step(&mut self, ms: f64) -> Result<(), String> {
		self.session.step(ms)?;
		self.session.unpark();
		// A page frame for each game frame: after its draw (`draw`), or now for a frame
		// that wasn't drawn (a loop that only steps), so the page never stands still.
		if !std::mem::replace(&mut self.drawn, false) {
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

	/// The game discarded a scene: the page gives back what it kept for it.
	pub fn discarded(&mut self) {
		self.session.host().release_memory();
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

	/// Give `key` to the page with `modifier` (the `KeyModifier` mask) held; whether the
	/// states get it too (not while the page has a text field focused).
	pub fn key(&mut self, key: &Key, modifier: i64) -> bool {
		let host = self.session.host();
		let modifiers = Modifiers {
			shift: modifier & 1 != 0,
			control: modifier & 2 != 0,
			alt: modifier & 4 != 0,
		};
		if let Some(code) = key.windows {
			host.key(code, key.down, modifiers);
		}
		if key.down {
			for c in key.text.iter().flat_map(|t| t.chars()) {
				host.character(c, modifiers);
			}
		}
		!self.session.text_field_focused()
	}

	/// Answer the procedural images (portraits) the page asked for with `compose`, given
	/// the engine and the path after `procedural/`.
	pub fn serve_procedural(&self, mut compose: impl FnMut(&GameEngine, &str) -> Option<Vec<u8>>) {
		for request in self.session.take_procedural() {
			let png = compose(self.session.engine(), &request.path);
			request.answer(png);
		}
	}

	/// What uploading the page's paints has cost so far.
	pub fn upload_stats(&self) -> bb_render::UploadStats {
		self.layer.upload_stats()
	}

	/// The bytes the page's texture takes on the GPU.
	pub fn layer_bytes(&self) -> u64 {
		self.layer.bytes()
	}

	/// Draw the page's latest paint over `target`.
	pub fn draw(&mut self, gpu: &Gpu, target: &RenderTarget) {
		// Only what the page repainted since the last frame.
		let layer = &mut self.layer;
		self.session.host().take_damage(|frame, damage| {
			if !damage.is_empty() {
				let regions: Vec<UiRegion> = damage
					.iter()
					.map(|r| UiRegion { x: r.x, y: r.y, width: r.width, height: r.height })
					.collect();
				layer.upload_regions(gpu, frame.width, frame.height, &frame.bgra, &regions);
			}
		});
		self.layer.draw(gpu, target);
		// The page's next frame, painted by the next game frame's draw (bb-ui's
		// `PageFrames::Game`).
		self.session.host().begin_frame();
		self.drawn = true;
	}
}
