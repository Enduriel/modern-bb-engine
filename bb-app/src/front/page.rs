//! The scripts with their real UI page, in the browser host (see [`super`]).

use std::path::Path;

use bb_engine::harness::GameEngine;
use bb_render::{Gpu, RenderTarget, UiLayer};
use bb_ui::host::{HostConfig, Modifiers, MouseButton, UiSession};

use super::Key;

/// How often the page paints, per second.
const PAGE_FRAME_RATE: i32 = 60;

pub struct Front {
	session: UiSession,
	layer: UiLayer,
	/// The paint last uploaded to the layer.
	painted: u64,
	/// Buttons whose press went to the page (bit `1 << id`): their release follows.
	captured: u32,
}

impl Front {
	/// Start the page and the scripts for the install at `data_dir`, and from the main
	/// menu start a new campaign on `seed`, as its Start button does. The campaign's
	/// opening event shows once the world is up.
	pub fn boot(gpu: &Gpu, data_dir: &Path, seed: &str) -> Result<Front, String> {
		let config = HostConfig {
			frame_rate: PAGE_FRAME_RATE,
			..HostConfig::default()
		};
		let session = UiSession::start(data_dir, config, false)?;
		session.boot_menu()?;
		session.start_campaign(seed)?;
		Ok(Front {
			session,
			layer: UiLayer::new(gpu),
			painted: 0,
			captured: 0,
		})
	}

	pub fn engine(&self) -> &GameEngine {
		self.session.engine()
	}

	/// The page and the game together, for checks.
	pub fn session(&self) -> &UiSession {
		&self.session
	}

	/// Advance the game by `ms` of real time, answering the page.
	pub fn step(&mut self, ms: f64) -> Result<(), String> {
		self.session.step(ms)
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

	/// Draw the page's latest paint over `target`.
	pub fn draw(&mut self, gpu: &Gpu, target: &RenderTarget) {
		let (layer, painted) = (&mut self.layer, &mut self.painted);
		self.session.host().with_frame(|frame| {
			if frame.paints != *painted {
				layer.upload(gpu, frame.width, frame.height, &frame.bgra);
				*painted = frame.paints;
			}
		});
		self.layer.draw(gpu, target);
	}
}
