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

/// At a battle's start, when the enemy's hit points outweigh the company's: every enemy's
/// cut so theirs come to 80% of the company's. The AI fights for the company, worse than
/// a player, and a player would have fled; the run is to go on. "" or what it did.
const EVEN_ODDS: &str = r#"(function() {
	local ours = 0.0, theirs = 0.0;
	foreach (faction in ::Tactical.Entities.getAllInstances())
		foreach (e in faction)
			if (e != null && e.isAlive() && !e.isNonCombatant())
			{
				if (e.isAlliedWithPlayer()) ours += e.getHitpoints(); else theirs += e.getHitpoints();
			}
	if (ours == 0 || theirs <= ours) return "";
	local scale = 0.8 * ours / theirs;
	foreach (faction in ::Tactical.Entities.getAllInstances())
		foreach (e in faction)
			if (e != null && e.isAlive() && !e.isAlliedWithPlayer())
				e.setHitpoints(::Math.max(1, (e.getHitpoints() * scale).tointeger()));
	return "the enemy's hit points cut to " + (scale * 100).tointeger() + "% (theirs " + theirs.tointeger() + ", ours " + ours.tointeger() + ")";
})()"#;

#[derive(Default)]
pub struct BattlePlay {
	frames: u32,
	weakened: bool,
	start: String,
}

impl Session<'_> {
	pub fn battle_tick(&mut self, started: bool) -> Result<(), End> {
		self.context("battle");
		if started {
			let state = self.eval("battle state", "::__soakState(true)");
			self.battle = BattlePlay { frames: 0, weakened: false, start: state.clone() };
			let _ = self.script("battle start", "if (\"__lastCombatResult\" in getroottable()) delete ::__lastCombatResult;");
			self.action(&format!("battle starts: {state}"));
			let odds = self.eval("even odds", EVEN_ODDS);
			if !odds.is_empty() {
				self.stats.keep_alive += 1;
				self.action(&format!("keep alive: {odds}"));
				Json::new("keep_alive").str("did", &odds).int("day", self.day_now).emit();
			}
		}
		self.battle.frames += TICK;
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
			.int("roster_after", roster)
			.int("day", self.day_now)
			.str("start", &self.battle.start)
			.emit();
	}
}
