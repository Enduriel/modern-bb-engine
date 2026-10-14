//! What a battle unit's sprite flags do, as the original's (disassembled 2026-10-14):
//! - every actor prevents occlusion, so a tree in front of a unit the player sees fades,
//!   but not one in front of a unit out of the player's sight
//!   (`SceneTacticalTile::unveils`);
//! - an actor's `arrow` sprite isn't colourised: the unit's colour natives leave it be,
//!   and its portrait draws its colourised sprites at full alpha while it's faded.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

/// A scripted battle against bandit raiders, the company left to the test.
const SETUP: &str = r#"
::__autoplayPlayer = function() {};
::__offerBattle <- function()
{
	::World.State.m.IsAutosaving = false;
	local st = ::World.State.get();
	local p = st.getLocalCombatProperties(st.getPlayer().getPos(), true);
	p.Entities = [];
	p.Parties = [];
	p.TerrainTemplate = "tactical.plains";
	p.CombatID = "Player";
	::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditRaiders, 60, ::World.FactionManager.getFactionOfType(::Const.FactionType.Bandits).getID());
	st.startScriptedCombat(p, true, true, true);
};
"#;

#[test]
fn a_units_sprite_flags_work_as_the_originals() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::install_autoplay(&engine).expect("autoplay");
	engine.run_script(SETUP, "setup").expect("setup");
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	engine.run_script("::__autoplayEngage = true; ::__offerBattle();", "engage").expect("engage");
	let mut ready = false;
	for _ in 0..6_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
		if player::ready(&engine).unwrap() {
			ready = true;
			break;
		}
	}
	assert!(ready, "a brother's turn");

	// Units unveil their tiles, as far as the player sees them.
	let scene = engine.tactical_scene();
	let (mut seen, mut hidden) = (0, 0);
	for e in scene.entities.iter().filter(|e| e.is_actor) {
		let Some(t) = e.tile.and_then(|t| scene.tiles.get(t)) else { continue };
		assert_eq!(t.unveils, t.visible, "unit {} on a {} tile", e.id, if t.visible { "seen" } else { "hidden" });
		if t.visible { seen += 1 } else { hidden += 1 }
	}
	eprintln!("{seen} units in sight, {hidden} out of it");
	assert!(seen > 0, "units in sight");
	// An enemy out of the player's sight doesn't fade what's in front of it.
	let moved = player::eval_i64(
		&engine,
		r#"(function() {
	local enemy = null;
	foreach (e in ::Tactical.Entities.getAllInstancesAsArray())
	{
		if (e.isAlive() && !e.isAlliedWithPlayer()) { enemy = e; break; }
	}
	for (local x = 0; x < ::Tactical.getMapSize().X; x++)
	{
		for (local y = 0; y < ::Tactical.getMapSize().Y; y++)
		{
			local t = ::Tactical.getTileSquare(x, y);
			if (t.IsEmpty && !t.IsVisibleForPlayer) { enemy.setTile(t); return t.ID; }
		}
	}
	return -1;
})()"#,
	)
	.unwrap()
	.unwrap();
	assert!(moved >= 0, "an unseen empty tile");
	let scene = engine.tactical_scene();
	let unseen = scene
		.entities
		.iter()
		.filter(|e| e.is_actor)
		.filter_map(|e| e.tile.and_then(|t| scene.tiles.get(t)))
		.filter(|t| !t.visible)
		.collect::<Vec<_>>();
	assert!(!unseen.is_empty(), "the enemy stands out of sight");
	assert!(unseen.iter().all(|t| !t.unveils), "and its tile doesn't unveil");
	// The path on show unveils its steps too (all in sight here, at the start).
	let path_tiles: Vec<usize> = scene.path.iter().map(|s| s.tile).collect();
	assert!(path_tiles.iter().all(|&t| scene.tiles[t].unveils == scene.tiles[t].visible));

	// setAlpha leaves the arrow alone; the portrait shows the faded unit at full alpha.
	let looks = player::eval_string(
		&engine,
		r#"(function() {
	local e = ::Tactical.TurnSequenceBar.getActiveEntity();
	local before = e.getSprite("arrow").Alpha;
	e.setAlpha(0);
	return e.getID() + " " + before + " " + e.getSprite("arrow").Alpha + " " + e.getSprite("body").Alpha;
})()"#,
	)
	.unwrap()
	.unwrap();
	let v: Vec<i64> = looks.split(' ').map(|x| x.parse().unwrap()).collect();
	let (id, arrow_before, arrow, body) = (v[0], v[1], v[2], v[3]);
	assert_eq!(body, 0, "setAlpha(0) clears the body");
	assert_eq!(arrow, arrow_before, "and leaves the arrow as it was");
	let portrait = engine.portrait(id).expect("a portrait");
	let body = portrait.sprites.iter().find(|s| s.name == "body").expect("a body sprite");
	assert_eq!(body.color >> 24, 0xff, "the portrait draws the body at full alpha");
}
