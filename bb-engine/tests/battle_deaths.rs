//! Brothers die in a battle, and the company goes back to the world map: the dead are
//! off the roster, and the world state's `onCombatFinished` updates every brother left
//! (morale, skills, items) without a script error, so the loading screen goes.
//!
//! A charmed brother who dies is off the roster too: the charm's `onDeath` gives him back
//! to the player before `actor.kill` asks whether the player controls him. (The test
//! harness's handover to the AI isn't a charm: a handed-over brother who dies stays on
//! the roster with his items' container cleared, and `onCombatFinished` then fails on him,
//! as the original's scripts do; hence `__autoplayDieAsPlayers`.)
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

/// A scripted battle against a few bandits, which the company wins.
const SETUP: &str = r#"
::__offerBattle <- function()
{
	::World.State.m.IsAutosaving = false;
	local st = ::World.State.get();
	local p = st.getLocalCombatProperties(st.getPlayer().getPos(), true);
	p.Entities = [];
	p.Parties = [];
	p.TerrainTemplate = "tactical.plains";
	p.CombatID = "Player";
	::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditRaiders, 40, ::World.FactionManager.getFactionOfType(::Const.FactionType.Bandits).getID());
	st.startScriptedCombat(p, true, true, true);
};
"#;

/// Fight a battle in which `deaths` (a script run once the units stand on the map)
/// kills brothers, the AI playing the company; then back to the world. The roster
/// shrinks by `fallen`.
fn battle_with_deaths(deaths: &str, fallen: i64) {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
		return;
	}
	let mut engine = harness::open_install(dir).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::install_autoplay(&engine).expect("autoplay");
	engine.run_script(SETUP, "setup").expect("setup");
	engine.run_script("::__autoplayFight = true; ::__autoplayDieAsPlayers = true;", "fight").expect("fight");
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	let before = player::eval_i64(&engine, "::World.getPlayerRoster().getSize()").unwrap().unwrap();
	engine.run_script("::__autoplayEngage = true; ::__offerBattle();", "engage").expect("engage");
	const TACTICAL: &str = "TacticalFromWorldState";
	let (mut fought, mut killed) = (false, false);
	for frame in 0..30_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
		let states = engine.root_state().state_names();
		if states.iter().any(|s| s == TACTICAL) {
			fought = true;
			let placed = "(\"State\" in ::Tactical) && ::Tactical.State != null && ::Tactical.TurnSequenceBar.getActiveEntity() != null ? 1 : 0";
			if !killed && player::eval_i64(&engine, placed).unwrap() == Some(1) {
				engine.run_script(deaths, "deaths").expect("deaths");
				killed = true;
			}
		} else if fought {
			break;
		}
	}
	assert!(fought && killed, "the battle never started");
	let states = engine.root_state().state_names();
	assert!(!states.iter().any(|s| s == TACTICAL), "the battle never ended: {states:?}");
	assert!(states.iter().any(|s| s == "WorldState"), "not back on the world map: {states:?}");
	for _ in 0..600 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	let back = player::eval_string(&engine, "\"loading \" + ::LoadingScreen.isVisible() + \", world \" + ::World.State.m.WorldScreen.isVisible()").unwrap().unwrap();
	let after = player::eval_i64(&engine, "::World.getPlayerRoster().getSize()").unwrap().unwrap();
	eprintln!("roster {before} -> {after}; {back}");
	assert_eq!(back, "loading false, world true", "back on the map");
	assert_eq!(after, before - fallen, "the fallen are off the roster");
}

#[test]
fn brothers_who_die_in_battle_leave_the_roster() {
	battle_with_deaths("local bros = ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player); bros[0].kill(); bros[1].kill();", 2);
}

#[test]
fn a_charmed_brother_who_dies_leaves_the_roster() {
	battle_with_deaths(
		r#"
		local bro = ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player)[0];
		// Given back to the player, so the charm is the only thing between him and the AI.
		bro.m.IsControlledByPlayer = true;
		local charm = ::new("scripts/skills/effects/charmed_effect");
		charm.setMasterFaction(::World.FactionManager.getFactionOfType(::Const.FactionType.Bandits).getID());
		bro.getSkills().add(charm);
		bro.kill();
		"#,
		1,
	);
}
