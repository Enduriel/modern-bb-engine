//! A webknecht's corpse twitches: its death spawns a body and a head detail on its tile
//! and moves them with `setOffset` every 10 ms of real time for about a second
//! (spider.nut `onCorpseEffect`). Details are of the sprite class in the original, so
//! they have it (see `sprite_class`); ours lacked it, and the twitch failed.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

const IN_TACTICAL: &str = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible() && ::Tactical.TurnSequenceBar.getActiveEntity() != null";

#[test]
fn a_webknechts_corpse_twitches() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		eprintln!("skipping: no Steam install");
		return;
	}
	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::unpause_world(&engine).expect("unpause");
	// Past the scenario's intro event (1 s of real time in).
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	engine
		.run_script(
			r#"
			::World.State.m.IsAutosaving = false;
			local st = ::World.State.get();
			local p = st.getLocalCombatProperties(st.getPlayer().getPos(), true);
			p.Entities = [];
			p.Parties = [];
			p.TerrainTemplate = "tactical.plains";
			p.CombatID = "Spiders";
			::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.Spiders, 60, ::World.FactionManager.getFactionOfType(::Const.FactionType.Beasts).getID());
			st.startScriptedCombat(p, false, false, false);
			"#,
			"battle",
		)
		.expect("battle");
	let mut frames = 0;
	while player::eval_i64(&engine, &format!("({IN_TACTICAL}) ? 1 : 0")).unwrap() != Some(1) {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
		frames += 1;
		assert!(frames < 6000, "the battle never began");
	}
	let killed = player::eval_i64(
		&engine,
		r#"(function() {
			foreach (faction in ::Tactical.Entities.getAllInstances())
				foreach (e in faction)
					if (e.getType() == ::Const.EntityType.Spider && e.isPlacedOnMap()) { e.kill(); return 1; }
			return 0;
		})()"#,
	)
	.unwrap();
	assert_eq!(killed, Some(1), "a webknecht to kill");
	// The twitch: about 1.2 s.
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	let corpse: Vec<(String, (f32, f32))> = engine
		.tactical_scene()
		.tiles
		.iter()
		.flat_map(|t| &t.details)
		.filter(|d| d.brush.starts_with("bust_spider_") && d.brush.ends_with("_dead"))
		.map(|d| (d.brush.clone(), d.offset))
		.collect();
	assert_eq!(corpse.len(), 2, "the body and the head: {corpse:?}");
	// It last set them 30 up, plus up to half a unit of its random twitch.
	for (brush, (x, y)) in &corpse {
		assert!(x.abs() <= 0.5 && (y - 30.0).abs() <= 1.0, "{brush} twitched to ({x}, {y})");
	}
}
