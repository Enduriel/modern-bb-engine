//! The brothers who come back from a battle are off its map. The original's
//! `Tactical.clearScene()` takes every unit off (probed: right after it, the roster's
//! `isPlacedOnMap()` is false, and their `onRemovedFromMap` hasn't run); world code then
//! takes them as out of battle (battle_standard_effect, captain_effect: a placed one
//! would reach `Tactical.State`, gone by then).
//!
//! Covered for a battle that ends because its enemies are all killed from outside (a
//! script's `kill()`), with the brothers the player's own, and for two battles in a row
//! with the AI fighting for them (the harness hands them back after each).
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

const IN_TACTICAL: &str = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible() && ::Tactical.TurnSequenceBar.getActiveEntity() != null";
const BACK_IN_WORLD: &str = "(!(\"State\" in ::Tactical) || ::Tactical.State == null) && ::World.isVisible() && !::LoadingScreen.isVisible()";

fn until(engine: &harness::GameEngine, cond: &str, frames: u32) {
	for _ in 0..frames {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap() == Some(1) {
			return;
		}
		harness::run_frame(engine, 1000.0 / 60.0).unwrap();
	}
	panic!("never: {cond}");
}

/// A campaign whose brothers won't fall, the scripted player (and with `fight`, the AI
/// fighting for the company) installed.
fn campaign(fight: bool) -> harness::GameEngine {
	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::install_autoplay(&engine).expect("autoplay");
	engine
		.run_script(
			&format!(
				"::__autoplayFight = {fight}; ::__autoplayDieAsPlayers = true; \
				 foreach (bro in ::World.getPlayerRoster().getAll()) {{ bro.getBaseProperties().Hitpoints = 900; bro.getSkills().update(); bro.setHitpoints(bro.getHitpointsMax()); }}"
			),
			"setup",
		)
		.expect("setup");
	engine
}

/// A scripted battle against a few bandits, through the combat dialog.
fn fight_bandits(engine: &harness::GameEngine) {
	engine
		.run_script(
			r#"
			::World.State.m.IsAutosaving = false;
			::__autoplayEngage = true;
			local st = ::World.State.get();
			local p = st.getLocalCombatProperties(st.getPlayer().getPos(), true);
			p.Entities = [];
			p.Parties = [];
			p.TerrainTemplate = "tactical.plains";
			p.CombatID = "Player";
			::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditRaiders, 40, ::World.FactionManager.getFactionOfType(::Const.FactionType.Bandits).getID());
			st.startScriptedCombat(p, true, true, true);
			"#,
			"battle",
		)
		.expect("battle");
	until(engine, IN_TACTICAL, 6000);
}

/// Every roster brother, placed or not: "name -" or "name PLACED".
fn roster(engine: &harness::GameEngine) -> String {
	player::eval_string(
		engine,
		"(function() { local s = \"\"; foreach (b in ::World.getPlayerRoster().getAll()) s += b.getName() + (b.isPlacedOnMap() ? \" PLACED\" : \" -\") + \", \"; return s; })()",
	)
	.unwrap()
	.unwrap()
}

#[test]
fn survivors_of_a_battle_won_from_outside_are_off_its_map() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		bb_engine::harness::skip("no Steam install");
		return;
	}
	let engine = campaign(false);
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	fight_bandits(&engine);
	assert!(roster(&engine).contains("PLACED"), "the brothers are on the battle map");
	engine
		.run_script(
			"foreach (faction in ::Tactical.Entities.getAllInstances()) foreach (e in clone faction) if (e != null && e.isAlive() && !e.isAlliedWithPlayer()) e.kill();",
			"kill them all",
		)
		.expect("kill");
	until(&engine, BACK_IN_WORLD, 6000);
	let after = roster(&engine);
	eprintln!("after: {after}");
	assert!(!after.contains("PLACED"), "nobody is still on the battle map: {after}");
	// World code that asks whether they're in a battle works.
	engine.run_script("foreach (b in ::World.getPlayerRoster().getAll()) { b.getSkills().update(); b.setHitpoints(b.getHitpointsMax()); }", "world").expect("world code on the survivors");
}

#[test]
fn the_ai_fights_two_battles_in_a_row_for_the_company() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		bb_engine::harness::skip("no Steam install");
		return;
	}
	let engine = campaign(true);
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	// Units held by rosters: the company, and the factions' characters.
	let before = engine.tactical_entities().unit_count();
	let mut units = Vec::new();
	for battle in 1..=2 {
		fight_bandits(&engine);
		until(&engine, BACK_IN_WORLD, 30_000);
		let after = roster(&engine);
		eprintln!("battle {battle}: {after}");
		assert!(!after.contains("PLACED"), "battle {battle}: nobody is still on the battle map: {after}");
		let handed = player::eval_i64(&engine, "(function() { local n = 0; foreach (b in ::World.getPlayerRoster().getAll()) if (!b.m.IsControlledByPlayer) n++; return n; })()").unwrap();
		assert_eq!(handed, Some(0), "battle {battle}: the brothers are the player's again");
		// The battle's enemies, dead and objects are let go with its scene: only the
		// rosters' units are left.
		units.push(engine.tactical_entities().unit_count());
		for _ in 0..120 {
			harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
		}
	}
	eprintln!("unit records before the battles: {before}, after each: {units:?}");
	assert!(units.iter().all(|&n| n == before), "unit records left over: {before} before, {units:?} after");
}
