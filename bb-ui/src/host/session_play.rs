//! A [`UiSession`] played live, in a window: frames of real time, a new campaign
//! started the way the menu starts it, and where the player's cursor is on the page.

use super::{Responder, UiSession};
use crate::json::Json;

/// A procedural image the page is waiting for (`coui://procedural/<path>`, e.g.
/// `tacticalentity(...)`).
pub struct ProceduralRequest {
	/// What follows `procedural/`, still percent-encoded.
	pub path: String,
	pub(super) responder: Responder,
}

impl ProceduralRequest {
	/// Answer with `png`, or the transparent placeholder.
	pub fn answer(self, png: Option<Vec<u8>>) {
		self.responder.respond_image(png);
	}
}

/// How the menu's "Start" (and the probe runner's campaign mode in the original)
/// starts a campaign, with the seed left as `{seed}`.
const START_CAMPAIGN: &str = r#"::RootState.get("MainMenuState").campaign_menu_module_onStartPressed({
	Name = "Probe Company",
	Banner = ::Const.PlayerBanners[0],
	Difficulty = 1,
	EconomicDifficulty = 1,
	BudgetDifficulty = 1,
	Ironman = false,
	ExplorationMode = false,
	GreaterEvil = ::Const.World.GreaterEvilType.Random,
	PermanentDestruction = false,
	Seed = {seed},
	StartingScenario = "scenario.early_access"
});"#;

impl UiSession {
	/// One frame of `ms` real time: the game's frame (the page parked), the page laid
	/// out at the UI scale, and the page's turn, its calls into the game answered.
	pub fn step(&self, ms: f64) -> Result<(), String> {
		self.park();
		self.game_frame(ms)?;
		self.follow_ui_scale();
		self.follow_fonts();
		self.pump();
		Ok(())
	}

	/// One frame of `ms` real time for a game loop that draws while the page works: the
	/// game's frame (the page parked), the page laid out at the UI scale, then the page
	/// let go on until the next step (or input) parks it, with what it sent so far
	/// handled. The page's turn is the time between frames, so it parks once a frame
	/// where [`step`](Self::step) and [`unpark`](Self::unpark) park it twice: what it
	/// calls on its own turn is answered as the next frame parks it (`park` serves calls
	/// while it waits), before that frame's scripts, as the original's UI runs between
	/// the game's frames.
	pub fn step_unparked(&self, ms: f64) -> Result<(), String> {
		self.park();
		self.game_frame(ms)?;
		self.follow_ui_scale();
		self.follow_fonts();
		self.unpark();
		for event in self.host().poll() {
			self.handle(event);
		}
		Ok(())
	}

	/// From the main menu, start a new campaign on map seed `seed` as the menu's Start
	/// does: the loading screen, then the world with its opening event. It runs over the
	/// next frames.
	pub fn start_campaign(&self, seed: &str) -> Result<(), String> {
		let seed = Json::Str(seed.to_string()).to_text();
		self.park();
		self.engine()
			.run_script(&START_CAMPAIGN.replace("{seed}", &seed), "start campaign")
	}

	/// The page pixel at the game's screen pixel `(x, y)`: the page is laid out at the
	/// screen size divided by the UI scale.
	pub fn to_page(&self, x: i64, y: i64) -> (i32, i32) {
		let (_, _, scale) = self.engine().video().screen();
		let scale = if scale > 0.0 { scale } else { 1.0 };
		((x as f64 / scale) as i32, (y as f64 / scale) as i32)
	}

	/// Whether the game's screen pixel `(x, y)` is over the UI: the painted page isn't
	/// transparent there. Coherent answered `UI.setCursorPosition` the same way, from
	/// the last frame (its click-through query). The page is painted at the
	/// screen's resolution, so its pixel is the screen's.
	pub fn is_over_ui(&self, x: i64, y: i64) -> bool {
		let (Ok(x), Ok(y)) = (i32::try_from(x), i32::try_from(y)) else {
			return false;
		};
		self.host().alpha_at(x, y) > 0
	}

	/// Keep the page's procedural images for [`UiSession::take_procedural`] to answer
	/// (`true`), or answer them with the transparent placeholder at once (`false`, the
	/// default).
	pub fn queue_procedural(&self, on: bool) {
		let mut queue = self.procedural.borrow_mut();
		if !on {
			for request in queue.take().into_iter().flatten() {
				request.answer(None);
			}
		} else if queue.is_none() {
			*queue = Some(Vec::new());
		}
	}

	/// The procedural images the page asked for since the last call.
	pub fn take_procedural(&self) -> Vec<ProceduralRequest> {
		self.procedural
			.borrow_mut()
			.as_mut()
			.map(std::mem::take)
			.unwrap_or_default()
	}
}
