//! The scripts with their UI's screens headless (see [`super`]).

use bb_engine::harness::{self, GameEngine};
use bb_render::{Gpu, RenderTarget};

use super::Key;
use crate::game::{Boot, Start};

pub struct Front {
	engine: GameEngine,
	profiling: Option<crate::script_profile::ScriptProfiling>,
}

impl Front {
	/// No browser: nothing to run (see the page's).
	pub fn pump_due(&self) -> Option<std::time::Instant> {
		None
	}

	pub fn pump(&self) {}

	pub fn set_pump_waker(&self, _waker: Option<super::Waker>) {}

	/// Boot the game as `boot` says, to the main menu or on into a new campaign, its
	/// opening event closed. (No page, so no DevTools.)
	pub fn boot(_gpu: &Gpu, boot: &Boot) -> Result<Front, String> {
		// A live clock behind Time.getExactTime, as the original's: the AI's background
		// work then spreads over frames. The battle camera glides and units hop over
		// time, as the original's.
		let archives = crate::game::archives(boot.data_dir, boot.mods).map_err(|e| e.to_string())?;
		let profiling = crate::script_profile::ScriptProfiling::from_env(&archives);
		let mut engine = harness::open_mounted(boot.data_dir, archives, |c| {
			c.exact_clock = squirrel_bb::ExactClock::Live;
			c.battle_camera = squirrel_bb::BattleCamera::Glides;
			c.battle_motion = squirrel_bb::BattleMotion::Timed;
			c.is_steam_build = boot.store.steam_build;
		})
		.map_err(|e| e.to_string())?;
		boot.store.attach(&engine);
		// An uncaught script error prints its call stack to stderr and log.html, as the
		// original logs it.
		squirrel_bb::report_script_errors(engine.vm_mut());
		engine.video().set_mode(boot.mode);
		if let Some(dir) = &boot.saves {
			engine.saves().set_directory(dir.clone());
		}
		if boot.sound {
			super::play_on_device(&engine, boot.data_dir, boot.mods);
		}
		harness::boot_menu(&engine)?;
		if let Some((cols, rows)) = boot.world_size {
			harness::set_world_size(&engine, cols, rows)?;
		}
		if let Start::NewCampaign(seed) = &boot.start {
			harness::prepare_new_campaign(&engine, seed)?;
			harness::start_new_campaign(&engine)?;
			// The menu's own Start hides the main menu as the world comes up
			// (main_menu_state.loading_screen_onScreenShown): without that, quitting to it
			// would find it still showing, never show it again, and the world would wait on it.
			engine.run_script("::RootState.get(\"MainMenuState\").hide();", "hide the main menu")?;
			engine.run_script("::World.State.m.MenuStack.popAll(true);", "close the opening event")?;
		}
		Ok(Front { engine, profiling })
	}

	pub fn engine(&self) -> &GameEngine {
		&self.engine
	}

	/// Advance the game by `ms` of real time.
	pub fn step(&mut self, ms: f64) -> Result<(), String> {
		let engine = &self.engine;
		let step = || -> Result<(), String> {
			engine.frame(ms)?;
			engine.pump_ui()?;
			Ok(())
		};
		match &mut self.profiling {
			Some(profiling) => profiling.step(engine.vm(), step),
			None => step(),
		}
	}

	/// Before the scripts run outside a step: no page to hold.
	pub fn park(&mut self) {}

	/// Show the FPS display, or hide it: there is no page to show it on.
	pub fn show_fps(&mut self, _shown: bool) -> Result<(), String> {
		Ok(())
	}

	/// Show the frame rate on the FPS display: there is none.
	pub fn set_fps(&mut self, _fps: u32) {}

	/// The game discarded a scene: no page to give anything back.
	pub fn discarded(&mut self) {}

	/// The cursor moved to the screen pixel `(x, y)`.
	pub fn cursor(&mut self, x: i64, y: i64) {
		self.engine.ui().set_cursor(x, y, false);
	}

	/// Whether the UI takes button `id`'s press or release: never.
	pub fn button(&mut self, _x: i64, _y: i64, _id: i64, _down: bool) -> bool {
		false
	}

	/// Whether the UI takes a turn of the wheel: never.
	pub fn wheel(&mut self, _x: i64, _y: i64, _up: bool) -> bool {
		false
	}

	/// Whether the states get `key` too: always.
	pub fn key(&mut self, _key: &Key, _modifier: i64) -> bool {
		true
	}

	/// Answer the page's procedural images: there is no page.
	pub fn serve_procedural(&self, _compose: impl FnMut(&GameEngine, &str) -> Option<Vec<u8>>) {}

	/// The page's paints: there is no page.
	pub fn paints(&self) -> u64 {
		0
	}

	/// Draw the UI over `target`: nothing to draw.
	pub fn draw(&mut self, _gpu: &Gpu, _target: &RenderTarget) {}

	/// No page, so no page texture.
	pub fn layer_bytes(&self) -> u64 {
		0
	}
}
