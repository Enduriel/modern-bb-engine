//! A non-number where a native wants a number reads as 0 instead of raising, as the
//! original's Sqrat bindings never raise there (they run the native with an
//! uninitialised value). Vanilla relies on it: the combat result screen's found loot
//! asks for its tooltip with no entity, and `tactical_queryUIItemTooltipData` calls
//! `Tactical.getEntityByID(null)` (found playing with mods: every loot tooltip raised
//! "expected an integer or float at parameter 1 (got Null)").
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

const IN_TACTICAL: &str = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible() && ::Tactical.TurnSequenceBar.getActiveEntity() != null";

#[test]
fn found_loot_tooltip_queries_entity_null() {
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
	harness::unpause_world(&engine).expect("unpause");
	for frame in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
	}
	harness::engage_nearest_hostile(&engine).expect("engage");
	let mut in_battle = false;
	for frame in 0..3_000 {
		if player::eval_i64(&engine, &format!("({IN_TACTICAL}) ? 1 : 0")).unwrap() == Some(1) {
			in_battle = true;
			break;
		}
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
	}
	assert!(in_battle, "the battle starts");

	engine
		.run_script(
			r#"
			::__r <- [];
			::__r.push(::Tactical.getEntityByID(null) == null);
			::__r.push(::World.getEntityByID(null) == null);
			local item = ::new("scripts/items/weapons/knife");
			::Tactical.CombatResultLoot.add(item);
			local events = ::new("scripts/ui/screens/tooltip/tooltip_events");
			local tip = events.tactical_queryUIItemTooltipData(null, item.getInstanceID(), "tactical-combat-result-screen.found-loot");
			::__r.push(typeof tip == "array" && tip.len() > 0);
			"#,
			"loot tooltip",
		)
		.expect("the found-loot tooltip query runs");
	let r = squirrel::SqTable::root(engine.vm()).get::<squirrel::SqArray>("__r").expect("results");
	let r: Vec<bool> = r.to_vec::<bool>();
	assert_eq!(r, [true, true, true], "[Tactical.getEntityByID(null) is null, World's too, a loot tooltip]");
}
