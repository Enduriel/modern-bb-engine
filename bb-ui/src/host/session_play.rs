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

	/// Fail the load, as a missing file fails (an image's `error`, an XHR's status 0).
	pub fn fail(self) {
		self.responder.fail();
	}
}

/// How long a game frame waits for the page to park before it goes on without its
/// scripts (see [`UiSession::step_unparked`]): far longer than a park takes in play
/// (~1 ms; ~25 ms when the page gives memory back after a discard), short enough that a
/// page busy for seconds leaves the window drawing. Waited once a park: the next frames
/// only look.
pub const FRAME_PARK_WAIT: std::time::Duration = std::time::Duration::from_millis(50);

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
	/// the game's frames. A page with nothing to do rests instead: it stays parked into
	/// the next frames, with none of its frames begun, until it has work
	/// ([`UiHost::let_page_run`](super::UiHost::let_page_run)).
	///
	/// A page that doesn't park at once (busy on its main thread: the memory release
	/// at a discard, a machine swapping) holds the frame's scripts, not the loop (see
	/// [`park_for_frame`](Self::park_for_frame)): the step returns without them, its
	/// calls into the game served, and the next step looks again, on the same park.
	/// Whether the game's frame ran.
	pub fn step_unparked(&self, ms: f64) -> Result<bool, String> {
		use squirrel_bb::frame_timing::span;
		let park = span("page park");
		let parked = self.park_for_frame();
		drop(park);
		if !parked {
			let _events = span("page unpark+events");
			for event in self.host().poll() {
				self.handle(event);
			}
			self.report_releases();
			return Ok(false);
		}
		let frame = span("engine frame");
		self.game_frame(ms)?;
		drop(frame);
		let follow = span("ui scale+fonts");
		self.follow_ui_scale();
		self.follow_fonts();
		drop(follow);
		let _events = span("page unpark+events");
		self.host().let_page_run();
		for event in self.host().poll() {
			self.handle(event);
		}
		self.report_releases();
		Ok(true)
	}

	/// Park the page for a game loop's scripts, if it parks soon: within
	/// [`FRAME_PARK_WAIT`] of being asked, or, while it gives memory back (which runs on
	/// its main thread), if it has parked already. Whether it did; if not, it is still
	/// asked, and the next call looks again ([`UiHost::try_park`](super::UiHost::try_park)).
	pub fn park_for_frame(&self) -> bool {
		let wait = if self.host().releasing() { std::time::Duration::ZERO } else { FRAME_PARK_WAIT };
		self.try_park(wait)
	}

	/// Park the page if it parks within `wait` of being asked (see
	/// [`UiHost::try_park`](super::UiHost::try_park)): whether it did. Its calls into
	/// the game meanwhile are served.
	pub fn try_park(&self, wait: std::time::Duration) -> bool {
		let (vm, bridge) = (self.engine().vm(), self.engine().ui());
		let parked = self
			.host()
			.try_park(wait, &mut |message| self.protocol().handle_message(vm, bridge, message));
		let parked = match parked {
			Ok(parked) => parked,
			Err(error) => {
				self.record("bridge", format!("the page didn't park: {error}"));
				// A page that's gone can't hold the game up.
				true
			}
		};
		self.report_notes();
		parked
	}

	/// The page's memory releases that finished: their time in the frame's
	/// (`page release`), and a line in log.html.
	fn report_releases(&self) {
		for released in self.host().take_releases() {
			squirrel_bb::frame_timing::add("page release", released.total);
			squirrel_bb::html_log::line(squirrel_bb::html_log::Class::Debug, "UI", released.describe());
		}
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
	/// frame's resolution, so its pixel is the screen's times the frame scale
	/// ([`set_frame_scale`](Self::set_frame_scale)).
	pub fn is_over_ui(&self, x: i64, y: i64) -> bool {
		let scale = self.frame_scale();
		let painted = |n: i64| (n as f64 * scale) as i64;
		let (Ok(x), Ok(y)) = (i32::try_from(painted(x)), i32::try_from(painted(y))) else {
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
