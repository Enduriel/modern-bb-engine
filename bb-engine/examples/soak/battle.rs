//! Battles: the AI fights both sides (the harness's autoplay, with `__autoplayFight` and
//! `__autoplayDieAsPlayers`); a long one is brought to an end by weakening the enemy.

use crate::record::Json;
use crate::session::{End, Session};

/// Frames per battle tick.
const TICK: u32 = 30;
/// Battle frames before a long battle's enemies are weakened (two minutes of its time),
/// and between weakenings after that.
const LONG_BATTLE: u32 = 7200;
const WEAKEN_EVERY: u32 = 1200;
/// Battle frames between progress lines in the action log.
const PROGRESS_EVERY: u32 = 6000;

const RESULTS: [&str; 5] = ["none", "enemy retreated", "enemy destroyed", "player retreated", "player destroyed"];

/// Every enemy at 1 hit point, every brother healed, rallied and rested: a battle that
/// drags on (AI misses, routs that never end) ends with the next blows, the company's.
const WEAKEN: &str = r#"
foreach (faction in ::Tactical.Entities.getAllInstances())
	foreach (e in faction)
		if (e != null && e.isAlive() && !e.isDying())
		{
			if (!e.isAlliedWithPlayer()) e.setHitpoints(1);
			else
			{
				e.setHitpoints(e.getHitpointsMax());
				if (e.getMoraleState() < ::Const.MoraleState.Steady) e.setMoraleState(::Const.MoraleState.Steady);
				e.setFatigue(0);
			}
		}
"#;

/// How many of the company stand on the field, of how many on the roster.
const DEPLOYED: &str = "::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player).len() + \" of \" + ::World.getPlayerRoster().getSize()";

#[derive(Default)]
pub struct BattlePlay {
	frames: u32,
	weakened: bool,
	start: String,
	/// The enemy's odds were evened this many times (`__soakEvenOdds`).
	evened: u32,
	/// Hostiles on the field at the last tick: more arriving get evened too.
	hostiles: i64,
	/// "n of m": the company on the field, of the roster.
	deployed: String,
}

impl Session<'_> {
	pub fn battle_tick(&mut self, started: bool) -> Result<(), End> {
		self.context("battle");
		if started {
			let state = self.eval("battle state", "::__soakState(true)");
			let deployed = self.eval("deployed", DEPLOYED);
			self.battle = BattlePlay { start: state.clone(), deployed: deployed.clone(), ..Default::default() };
			let _ = self.script("battle start", "if (\"__lastCombatResult\" in getroottable()) delete ::__lastCombatResult;");
			self.action(&format!("battle starts ({deployed} of the company on the field): {state}"));
			// A company that keeps fielding a lone brother isn't playing the game.
			let mut n = deployed.split(" of ").map(|x| x.trim().parse::<i64>().unwrap_or(0));
			if let (Some(field), Some(roster)) = (n.next(), n.next())
				&& field <= 1 && roster >= 2
			{
				self.stats.battles_lone += 1;
			}
		}
		// The odds, at the start and whenever more enemies arrive.
		let hostiles = self.eval_i64("hostiles", "::Tactical.Entities.getHostilesNum()").unwrap_or(0);
		if started || hostiles > self.battle.hostiles {
			let odds = self.eval("even odds", "::__soakEvenOdds()");
			if !odds.is_empty() {
				self.battle.evened += 1;
				self.stats.keep_alive += 1;
				self.action(&format!("keep alive: {odds}"));
				Json::new("keep_alive").str("did", &odds).int("day", self.day_now).emit();
			}
		}
		self.battle.hostiles = hostiles;
		self.battle.frames += TICK;
		if self.battle.frames % PROGRESS_EVERY < TICK {
			self.observe();
			let state = self.eval("battle state", "::__soakState(true)");
			self.action(&format!("battle at {} frames: {state}", self.battle.frames));
		}
		if self.battle.frames >= LONG_BATTLE && (self.battle.frames - LONG_BATTLE) % WEAKEN_EVERY < TICK {
			self.weaken();
		}
		self.frames(TICK)
	}

	/// Weaken the enemy of the battle under way (recorded once per battle).
	pub fn weaken(&mut self) {
		if self.script("weaken", &format!("if (::__soakInTactical()) {{ {WEAKEN} }}")).is_ok() && !self.battle.weakened {
			self.battle.weakened = true;
			self.stats.battles_weakened += 1;
			self.action(&format!("battle long ({} frames): the enemy weakened", self.battle.frames));
		}
	}

	/// The battle just left: its result, for the record.
	pub fn battle_over(&mut self) {
		self.observe();
		let result = self.eval_i64("result", "(\"__lastCombatResult\" in getroottable()) ? ::__lastCombatResult : -1").unwrap_or(-1);
		let _ = self.script("result", "if (\"__lastCombatResult\" in getroottable()) delete ::__lastCombatResult;");
		self.stats.battles += 1;
		self.world.after_battle = true;
		if result == 1 || result == 2 {
			self.stats.battles_won += 1;
		}
		let name = usize::try_from(result).ok().and_then(|r| RESULTS.get(r)).copied().unwrap_or("unknown (no result screen left)");
		let roster = self.eval_i64("roster", "(\"State\" in ::World) && ::World.State != null ? ::World.getPlayerRoster().getSize() : -1").unwrap_or(-1);
		self.action(&format!("battle over after {} frames: {name}, {roster} brothers left", self.battle.frames));
		Json::new("battle")
			.int("frames", self.battle.frames as i64)
			.int("result", result)
			.str("result_name", name)
			.int("weakened", self.battle.weakened as i64)
			.int("evened", self.battle.evened as i64)
			.str("deployed", &self.battle.deployed)
			.int("roster_after", roster)
			.int("day", self.day_now)
			.str("start", &self.battle.start)
			.emit();
	}
}
