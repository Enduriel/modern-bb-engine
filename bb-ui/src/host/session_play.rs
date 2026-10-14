//! A [`UiSession`] played live, in a window: frames of real time, a new campaign
//! started the way the menu starts it, and where the player's cursor is on the page.

use super::UiSession;
use crate::json::Json;

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
	/// One frame of `ms` real time: the game's frame, the page laid out at the UI
	/// scale, and the page's calls into the game answered.
	pub fn step(&self, ms: f64) -> Result<(), String> {
		self.engine().frame(ms)?;
		self.follow_ui_scale();
		self.pump();
		Ok(())
	}

	/// From the main menu, start a new campaign on map seed `seed` as the menu's Start
	/// does: the loading screen, then the world with its opening event. It runs over the
	/// next frames.
	pub fn start_campaign(&self, seed: &str) -> Result<(), String> {
		let seed = Json::Str(seed.to_string()).to_text();
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
	/// the last frame (its click-through query).
	pub fn is_over_ui(&self, x: i64, y: i64) -> bool {
		let (px, py) = self.to_page(x, y);
		self.host().alpha_at(px, py) > 0
	}

	/// Whether a text field in the page has the keyboard (keys typed there aren't the
	/// game's hotkeys).
	pub fn text_field_focused(&self) -> bool {
		let focused = self.eval(
			"(function () {
	var e = document.activeElement;
	return !!e && (e.tagName === 'INPUT' || e.tagName === 'TEXTAREA' || e.isContentEditable);
})()",
		);
		matches!(focused, Ok(Json::Bool(true)))
	}
}
