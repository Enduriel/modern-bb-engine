//! A battle whose road walks off the map starts. Vanilla's `tactical_template.addRoads`
//! guards its north-south walk's west step with `roadY` instead of `roadX`, so now and
//! then the road reaches `roadX = -1` and the road tile asks for
//! `Tactical.getTileSquare(-1, y)` (the gameplay soak met it once in about 1,100
//! battles). The original reads off its column array there and crashes the game
//! (probed); ours gives the nearest square on the map, with a warning, and the battle
//! starts with its road down the west edge. `isValidTileSquare` stays strict.
//!
//! The walk is steered off the map (`testdata/offmap_road.nut`). Skips without the
//! Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};
use squirrel::{SqArray, SqTable};

const BATTLE_UP: &str = "(\"State\" in ::Tactical) && ::Tactical.State != null && ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Enemy).len() != 0 && ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Enemy)[0].isPlacedOnMap()";

#[test]
fn a_road_walking_off_the_map_leaves_the_battle_intact() {
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
	harness::run_past_intro(&engine).expect("past the intro");
	engine.run_script(include_str!("testdata/offmap_road.nut"), "offmap_road.nut").expect("the battle is offered");

	let mut up = false;
	for frame in 0..3_000 {
		if player::eval_i64(&engine, &format!("({BATTLE_UP}) ? 1 : 0")).unwrap() == Some(1) {
			up = true;
			break;
		}
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
	}
	assert!(up, "the battle starts");

	let root = SqTable::root(engine.vm());
	let offmap = root.get::<SqTable>("__offmap").expect("the record");
	assert_eq!(offmap.get::<i64>("AddRoads"), Some(1), "the plains' addRoads ran once");
	// The walk reached x = -1 at row 18 and went south from there: each read gave the
	// edge square of its row.
	let reads: Vec<String> = offmap.get::<SqArray>("Reads").expect("reads").to_vec::<String>();
	let expected: Vec<String> = (18..32).map(|y| format!("0,{y}")).collect();
	assert_eq!(reads, expected);
	// The road runs down the west edge from there, and nothing off the map is valid.
	let edge = player::eval_string(
		&engine,
		"(function() { local s = \"\"; for (local y = 0; y < 32; y++) if (::Tactical.getTileSquare(0, y).Subtype == ::Const.Tactical.TerrainSubtype.CobblestoneRoad) s += y + \" \"; return s; })()",
	)
	.unwrap()
	.unwrap();
	assert!(edge.contains("18 ") && edge.contains("31 "), "road on the west edge: {edge}");
	assert_eq!(
		player::eval_i64(&engine, "(!::Tactical.isValidTileSquare(-1, 28) && !::Tactical.isValidTileSquare(32, 5) && !::Tactical.isValidTileSquare(5, -1)) ? 1 : 0").unwrap(),
		Some(1)
	);
	// Any square off the map gives the nearest one on it.
	let nearest = player::eval_string(
		&engine,
		"(function() { local s = \"\"; foreach (c in [[-1, 28], [32, 5], [5, -1], [5, 32], [-100, 1000]]) { local t = ::Tactical.getTileSquare(c[0], c[1]); s += t.SquareCoords.X + \",\" + t.SquareCoords.Y + \" \"; } return s; })()",
	)
	.unwrap()
	.unwrap();
	assert_eq!(nearest, "0,28 31,5 5,0 5,31 0,31 ");
}
