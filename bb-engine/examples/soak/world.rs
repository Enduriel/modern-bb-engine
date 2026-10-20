//! The world map: starting campaigns, the company's moves (travel, contracts, hunting),
//! town visits, keeping the company going, and game over.

use crate::record::{Json, clip};
use crate::session::{End, Phase, Session};

/// The main menu's screen.
const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
pub const AT_MENU: &str = "::RootState.get(\"WorldState\") == null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible() && !::RootState.get(\"MainMenuState\").m.MainMenuScreen.isAnimating()";
pub const IN_WORLD: &str = "::__soakInWorld()";
/// Unpause, unless a screen has the world stopped (as a player presses Space).
const UNPAUSE: &str = "if (::World.State.isPaused() && !::World.State.m.EventScreen.isVisible() && !::World.State.m.MenuStack.hasBacksteps()) { ::World.State.setAutoPause(false); ::World.State.setPause(false); }";

/// Frames per world tick.
const TICK: u32 = 30;
/// Ticks between keep-alive checks.
const KEEP_ALIVE_EVERY: u32 = 20;
/// Ticks a march may take before the company thinks again.
const MARCH_TICKS: u32 = 400;
/// Ticks a contract may stay active before it is given up (cancelled).
const CONTRACT_DAYS: i64 = 20;

#[derive(Default)]
pub struct WorldPlay {
	ticks: u32,
	/// Ticks since the last decision while marching.
	marching: u32,
	/// The town visit's steps left, and ticks spent leaving.
	town_steps: Vec<&'static str>,
	leaving: u32,
	/// The active contract's id and the day it was taken.
	contract: Option<(String, i64)>,
	campaigns: u32,
	/// A battle just ended: keep the company going at the next tick.
	pub after_battle: bool,
}

impl Session<'_> {
	/// Start a campaign from the main menu's New Campaign page, with the origin
	/// `--scenario` and the next seed (MSU's new-campaign settings, if it asks, kept).
	pub fn start_campaign(&mut self) -> Result<(), String> {
		self.world.campaigns += 1;
		let k = self.world.campaigns;
		self.campaign_seed = if k == 1 { self.o.seed.clone() } else { format!("{}-c{k}", self.o.seed) };
		let seed = self.campaign_seed.clone();
		self.context("new campaign");
		let stalled = |e: End| match e {
			End::Stalled(why) => why,
			End::Deadline => "time up".into(),
		};
		if !self.until(1200, &format!("{MENU}.isVisible() && !{MENU}.isAnimating() && !::LoadingScreen.isVisible()")).map_err(stalled)? {
			return Err("the main menu didn't show".into());
		}
		self.script("new campaign", &format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"))?;
		self.frames(2).map_err(stalled)?;
		self.script(
			"start",
			&format!(
				"{MENU}.getNewCampaignMenuModule().onStartButtonPressed({{
					companyName = \"Soak Company\", banner = \"banner_04\",
					combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
					ironman = false, explorationDifficulty = false,
					lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
					seed = \"{seed}\", origin = \"{}\"
				}});",
				self.o.scenario
			),
		)?;
		let msu = "(\"MSU\" in ::getroottable()) && ::MSU.SettingsScreen.isVisible()";
		let mut started = false;
		for _ in 0..1200 {
			if self.holds("started", IN_WORLD) {
				started = true;
				break;
			}
			if self.holds("msu settings", msu) {
				self.script("MSU settings", "::MSU.SettingsScreen.onSaveButtonPressed({});")?;
			}
			self.frames(5).map_err(stalled)?;
		}
		if !started {
			let state = self.eval("state", "::__soakState(true)");
			return Err(format!("the campaign (seed {seed}, {}) didn't start: {state}", self.o.scenario));
		}
		self.script(
			"autoplay flags",
			"::__autoplayEngage = true; ::__autoplayFight = true; ::__autoplayDieAsPlayers = true; ::World.State.m.IsAutosaving = false;",
		)?;
		self.stats.campaigns += 1;
		let day = self.eval_i64("day", "::World.getTime().Days").unwrap_or(0);
		self.day_from = day;
		self.day_now = day;
		self.next_save_day = day + self.o.save_every_days;
		self.world.contract = None;
		let state = self.eval("state", "::__soakState(false)");
		self.action(&format!("campaign {k} started: seed {seed}, {}: {state}", self.o.scenario));
		Json::new("campaign_start").str("campaign_seed", &seed).str("scenario", &self.o.scenario).str("state", &state).emit();
		Ok(())
	}

	/// Count the days, and note the autoplay's choices.
	pub fn observe(&mut self) {
		if let Some(day) = self.eval_i64("day", "::World.getTime().Days")
			&& day > self.day_now
		{
			self.stats.days += day - self.day_now;
			self.day_now = day;
		}
		let notes = self.eval("notes", "::__soakDrain()");
		for note in notes.split('\x1e').filter(|n| !n.is_empty()) {
			if note.starts_with("event ") || note.starts_with("contract ") {
				self.stats.events_answered += 1;
			}
			self.action(note);
		}
	}

	pub fn world_tick(&mut self, was: Phase) -> Result<(), End> {
		self.context("world");
		self.observe();
		self.world.ticks += 1;
		let speed = self.o.world_speed;
		// The speed a player picks (as the fast-time button does, which a pause keeps and an
		// unpause restores); a paused world stays paused.
		let _ = self.script(
			"unpause",
			&format!("::World.State.m.LastWorldSpeedMult = {speed:?}; {UNPAUSE} if (!::World.State.isPaused() && ::World.getSpeedMult() != {speed:?}) ::World.setSpeedMult({speed:?});"),
		);
		if self.world.ticks % KEEP_ALIVE_EVERY == 0 || std::mem::take(&mut self.world.after_battle) {
			self.keep_alive();
		}
		self.track_contract();
		if self.day_now >= self.next_save_day && was == Phase::World {
			self.save_and_load()?;
			return Ok(());
		}
		self.decide();
		self.frames(TICK)
	}

	/// The company's next move, when it has none (or has marched too long).
	fn decide(&mut self) {
		let busy = self.holds("busy", "::__soakBusy()");
		if busy {
			self.world.marching += 1;
			if self.world.marching < MARCH_TICKS {
				// Now and then, a party in sight on the way is worth a fight.
				if self.world.marching % 15 == 0 && self.rand(4) == 0 {
					let note = self.eval("hunt", "(function() { local w = ::World.State; w.m.AutoEnterLocation = null; local s = ::__soakHunt(350.0); return s; })()");
					if !note.is_empty() {
						self.action(&format!("on the way: {note}"));
					}
				}
				return;
			}
			self.action("marched too long: thinking again");
			let _ = self.script("stop", "::World.State.getPlayer().setPath(null); ::World.State.m.AutoEnterLocation = null; ::World.State.m.AutoAttack = null;");
		}
		self.world.marching = 0;
		let has_contract = self.world.contract.is_some();
		let roll = self.rand(100);
		let note = if has_contract && roll < 60 {
			self.eval("pursue", "::__soakPursue()")
		} else if roll < 30 {
			self.eval("hunt", "::__soakHunt(900.0)")
		} else {
			String::new()
		};
		let note = if note.is_empty() || note.contains("no path") || note.contains("nowhere") {
			if !note.is_empty() {
				self.action(&note);
			}
			self.eval("travel", "::__soakTravel()")
		} else {
			note
		};
		if note.is_empty() {
			self.action("nothing to do: waiting");
		} else {
			self.action(&note);
		}
	}

	/// Follow the active contract: taken, ended, or given up after [`CONTRACT_DAYS`].
	fn track_contract(&mut self) {
		let id = self.eval("contract", "(function() { local c = ::World.Contracts.getActiveContract(); return c == null ? \"\" : c.getType() + \"#\" + c.getID(); })()");
		match (&self.world.contract, id.is_empty()) {
			(None, false) => {
				self.stats.contracts_taken += 1;
				self.action(&format!("contract taken: {id}"));
				Json::new("contract").str("event", "taken").str("contract", &id).int("day", self.day_now).emit();
				self.world.contract = Some((id, self.day_now));
			}
			(Some((old, _)), true) => {
				let old = old.clone();
				self.stats.contracts_ended += 1;
				let done = self.eval_i64("contracts done", "::World.Contracts.getContractsFinished()").unwrap_or(-1);
				self.action(&format!("contract ended: {old} (finished in all: {done})"));
				Json::new("contract").str("event", "ended").str("contract", &old).int("day", self.day_now).int("finished_in_all", done).emit();
				self.world.contract = None;
			}
			(Some((_, since)), false) if self.day_now - since > CONTRACT_DAYS => {
				self.action(&format!("contract {id} active for {CONTRACT_DAYS} days: cancelled"));
				let _ = self.script("cancel contract", "::World.Contracts.finishActiveContract(true);");
			}
			_ => {}
		}
	}

	/// Top up what the company needs to go on (recorded: it changes the game).
	fn keep_alive(&mut self) {
		let note = self.eval("keep alive", "::__soakKeepAlive()");
		if !note.is_empty() {
			self.stats.keep_alive += 1;
			self.action(&format!("keep alive:{note}"));
			Json::new("keep_alive").str("did", note.trim()).int("day", self.day_now).emit();
		}
	}

	/// A town visit: a contract (maybe), the market, the crowd, then leave.
	pub fn town_tick(&mut self, entered: bool) -> Result<(), End> {
		self.context("town");
		self.observe();
		if entered {
			self.stats.towns_entered += 1;
			let name = self.eval("town", "::World.State.getCurrentTown().getName()");
			self.action(&format!("entered {name}"));
			let mut steps = vec!["leave"];
			if self.rand(3) != 0 {
				steps.push("hire");
			}
			if self.rand(4) != 0 {
				steps.push("market");
			}
			if self.rand(10) < 7 {
				steps.push("contract");
			}
			self.world.town_steps = steps;
			self.world.leaving = 0;
			self.world.marching = 0;
		}
		// Wait for screens over the town (a contract's) to close.
		if self.holds("event", "::World.State.m.EventScreen.isVisible()") {
			return self.frames(10);
		}
		match self.world.town_steps.last().copied() {
			Some("leave") | None => {
				self.world.leaving += 1;
				if self.world.leaving > 40 {
					self.action("leaving the town takes too long: closing every menu");
					let _ = self.script("leave", "::World.State.m.MenuStack.popAll(true);");
				} else {
					let _ = self.eval("leave", "::__soakTownStep(\"leave\")");
				}
			}
			Some(step) => {
				self.world.town_steps.pop();
				let note = self.eval(step, &format!("::__soakTownStep(\"{step}\")"));
				if !note.is_empty() {
					for part in note.split(", ").filter(|p| !p.ends_with("(refused)")) {
						self.stats.sold += part.starts_with("sold ") as i64;
						self.stats.bought += part.starts_with("bought ") as i64;
					}
					if note.contains("-> hired") {
						self.stats.hired += 1;
					}
					if note.contains("contract offered") {
						self.stats.contracts_offered += 1;
					}
					self.action(&note);
				}
			}
		}
		self.frames(15)
	}

	/// Game over: record it, quit to the main menu, start the next campaign.
	pub fn game_over(&mut self) -> Result<(), End> {
		self.observe();
		self.stats.games_over += 1;
		let state = self.eval("state", "::__soakState(true)");
		self.action(&format!("GAME OVER: {state}"));
		Json::new("game_over").str("campaign_seed", &self.campaign_seed).int("days", self.day_now - self.day_from).str("state", &state).emit();
		let _ = self.script("quit", "::World.State.game_finish_dialog_onQuitPressed();");
		if !self.until(6000, AT_MENU)? {
			let state = self.eval("state", "::__soakState(true)");
			return Err(End::Stalled(format!("quitting after game over: {}", clip(&state, 300))));
		}
		self.start_campaign().map_err(End::Stalled)
	}

	/// At the main menu without having asked for it: start a campaign.
	pub fn at_menu(&mut self) -> Result<(), End> {
		if !self.until(600, AT_MENU)? {
			return self.frames(10);
		}
		self.action("at the main menu: starting a campaign");
		Json::new("unexpected_menu").emit();
		self.start_campaign().map_err(End::Stalled)
	}
}
