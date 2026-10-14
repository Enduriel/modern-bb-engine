//! A unit's move in a battle, paced as the windowed game paces it (`BattleMotion::Timed`):
//! a brother sent two tiles away hops there over several frames as the original's
//! navigator moves it. It lifts off its tile, flies in an arc to above the destination
//! (taking that tile as it leaves), lands, and only then is done; every frame shows it
//! somewhere on that way. With faster movement off, a hop takes its full time.
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
foreach (s in [::Settings.getGameplaySettings(), ::Settings.getTempGameplaySettings()]) s.FasterPlayerMovement = false;
"#;

/// An empty tile two steps from the acting unit, on its level: "x y".
const TARGET: &str = r#"(function() {
	local e = ::Tactical.TurnSequenceBar.getActiveEntity(), my = e.getTile();
	for (local d = 0; d < 6; d++)
	{
		if (!my.hasNextTile(d)) continue;
		local a = my.getNextTile(d);
		if (!a.IsEmpty || a.Level != my.Level || !a.hasNextTile(d)) continue;
		local b = a.getNextTile(d);
		if (b.IsEmpty && b.Level == my.Level) return b.SquareCoords.X + " " + b.SquareCoords.Y;
	}
	return "";
})()"#;

/// The acting unit's position, its tile, and virtual seconds.
const SAMPLE: &str = r#"(function() {
	local e = ::__mover, p = e.getPos();
	return p.X + " " + p.Y + " " + e.getTile().ID + " " + ::Time.getVirtualTimeF();
})()"#;

fn numbers(s: &str) -> Vec<f64> {
	s.split(' ').map(|x| x.parse().unwrap()).collect()
}

#[test]
fn a_unit_hops_to_its_destination_over_time() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install_with(dir, |c| {
		c.battle_motion = squirrel_bb::BattleMotion::Timed;
		c.battle_camera = squirrel_bb::BattleCamera::Glides;
	})
	.expect("open install");
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
	let target = player::eval_string(&engine, TARGET).unwrap().unwrap();
	let target = numbers(&target);
	let (x, y) = (target[0] as i64, target[1] as i64);
	engine.run_script("::__mover <- ::Tactical.TurnSequenceBar.getActiveEntity();", "mover").unwrap();
	let id = player::eval_i64(&engine, "::__mover.getID()").unwrap().unwrap();
	let from = numbers(&player::eval_string(&engine, SAMPLE).unwrap().unwrap());
	let dest = player::eval_string(&engine, &format!("::Tactical.getTileSquare({x}, {y}).Pos.X + \" \" + ::Tactical.getTileSquare({x}, {y}).Pos.Y + \" \" + ::Tactical.getTileSquare({x}, {y}).ID")).unwrap().unwrap();
	let dest = numbers(&dest);
	assert!(player::move_to(&engine, x, y).unwrap(), "set off to ({x}, {y})");

	// Every frame until it has landed.
	let mut frames = Vec::new();
	for _ in 0..600 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
		let s = numbers(&player::eval_string(&engine, SAMPLE).unwrap().unwrap());
		let flying = engine.tactical_scene().entities.iter().find(|e| e.id == id).is_some_and(|e| e.flying);
		let moving = player::eval_i64(&engine, "::Tactical.getNavigator().isTravelling(::__mover) ? 1 : 0").unwrap() == Some(1);
		frames.push((s, flying, moving));
		if !moving {
			break;
		}
	}
	let (last, _, moving) = frames.last().unwrap();
	assert!(!moving, "landed");
	assert_eq!((last[0], last[1], last[2]), (dest[0], dest[1], dest[2]), "on the destination tile");
	let took = last[3] - from[3];
	// 0.14 s up, 0.18 s per 126 units of flight (two tiles: about 252), 0.23 s down.
	assert!(took > 0.6 && took < 0.9, "the hop took {took:.3} s");
	assert!(frames.len() > 30, "over {} frames", frames.len());

	let (a, b) = ((from[0], from[1]), (dest[0], dest[1]));
	let mut phase = 0;
	let mut peak = f64::MIN;
	for (s, flying, moving) in &frames {
		if !moving {
			break;
		}
		let (px, py, tile) = (s[0], s[1], s[2]);
		match (phase, *flying) {
			(0, false) => {
				// Lifting off: over the start tile, which it still holds.
				assert_eq!(px, a.0, "lifting straight up");
				assert!(py >= a.1 && py <= a.1 + 40.0 + 1e-3, "at most 40 up ({})", py - a.1);
				assert_eq!(tile, from[2], "still on its tile");
			}
			(0 | 1, true) => {
				phase = 1;
				// Flying: holds the destination; between the two, above both.
				assert_eq!(tile, dest[2], "the destination is taken at lift-off's end");
				assert!(px >= a.0.min(b.0) - 1e-3 && px <= a.0.max(b.0) + 1e-3, "between the tiles across");
				assert!(py >= a.1.min(b.1) + 40.0 - 1e-3, "in the air");
				peak = peak.max(py);
			}
			(1 | 2, false) => {
				phase = 2;
				// Landing over the destination.
				assert_eq!(px, b.0, "dropping straight down");
				assert!(py >= b.1 - 1e-3 && py <= b.1 + 50.0 + 1e-3, "at most 50 up ({})", py - b.1);
			}
			other => panic!("out of order: {other:?}"),
		}
	}
	assert_eq!(phase, 2, "lifted off, flew and landed");
	// The arc: 3/4 of a quarter of the distance across over the line (here 45 at most).
	let across = (b.0 - a.0).abs();
	let arc = (0.25 * across).min(60.0) * 0.75;
	let line = (a.1 + 40.0 + b.1 + 50.0) / 2.0;
	assert!(peak > line + arc * 0.8, "arcs over the line: peak {peak:.1}, line {line:.1}, arc {arc:.1}");
}
