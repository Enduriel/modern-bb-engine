//! The scripts with their UI's screens headless (see [`super`]).

use std::path::Path;

use bb_engine::harness::{self, GameEngine};
use bb_render::{Gpu, RenderTarget};

use super::Key;

pub struct Front {
	engine: GameEngine,
}

impl Front {
	/// Boot the install at `data_dir` into a new campaign on `seed` (on a world map of
	/// `world_size` columns and rows, if given), with `sound` on the device or silent.
	pub fn boot(_gpu: &Gpu, data_dir: &Path, seed: &str, sound: bool, world_size: Option<(i64, i64)>) -> Result<Front, String> {
		// A live clock behind Time.getExactTime, as the original's: the AI's background
		// work then spreads over frames. The battle camera glides and units hop over
		// time, as the original's.
		let engine = harness::open_install_with(data_dir, |c| {
			c.exact_clock = squirrel_bb::ExactClock::Live;
			c.battle_camera = squirrel_bb::BattleCamera::Glides;
			c.battle_motion = squirrel_bb::BattleMotion::Timed;
		})
		.map_err(|e| e.to_string())?;
		if sound {
			super::play_on_device(&engine, data_dir);
		}
		harness::boot_menu(&engine)?;
		if let Some((cols, rows)) = world_size {
			harness::set_world_size(&engine, cols, rows)?;
		}
		harness::prepare_new_campaign(&engine, seed)?;
		harness::start_new_campaign(&engine)?;
		engine.run_script("::World.State.m.MenuStack.popAll(true);", "close the opening event")?;
		Ok(Front { engine })
	}

	pub fn engine(&self) -> &GameEngine {
		&self.engine
	}

	/// Advance the game by `ms` of real time.
	pub fn step(&mut self, ms: f64) -> Result<(), String> {
		self.engine.frame(ms)?;
		self.engine.pump_ui()?;
		Ok(())
	}

	/// Before the scripts run outside a step: no page to hold.
	pub fn park(&mut self) {}

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

	/// Draw the UI over `target`: nothing to draw.
	pub fn draw(&mut self, _gpu: &Gpu, _target: &RenderTarget) {}
}
