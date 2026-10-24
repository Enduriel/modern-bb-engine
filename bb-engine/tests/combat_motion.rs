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

/// A battle paced as the windowed game paces it, at a brother's turn; `None` (skipped)
/// without the Steam install.
fn timed_battle() -> Option<harness::GameEngine> {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
		return None;
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
	Some(engine)
}

#[test]
fn a_unit_hops_to_its_destination_over_time() {
	let Some(engine) = timed_battle() else { return };
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

/// Two brothers next to each other, the one's id, then the other's.
const PAIR: &str = r#"(function() {
	local players = ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player);
	foreach (x in players)
		foreach (y in players)
			if (x != y && x.getTile().getDistanceTo(y.getTile()) == 1 && x.getTile().Level == y.getTile().Level)
			{
				::__pair <- [x, y];
				::__tiles <- [x.getTile(), y.getTile()];
				return x.getID() + " " + y.getID();
			}
	return "";
})()"#;

/// Per unit of the pair: placed, travelling, x, y; then virtual seconds.
const PAIR_SAMPLE: &str = r#"(function() {
	local s = "";
	foreach (e in ::__pair)
		s += (e.isPlacedOnMap() ? 1 : 0) + " " + (::Tactical.getNavigator().isTravelling(e) ? 1 : 0) + " " + e.getPos().X + " " + e.getPos().Y + " ";
	return s + ::Time.getVirtualTimeF();
})()"#;

/// Rotation's swap (`switchEntities(a, b, onDone, tag, 1.0)`) in view, as the original
/// does it (`0x4ca160`, update `0x4ca990`; probed 2026-10-24). Both units leave their
/// tiles at once (`getPos` reads (0, 0) until the next update) but stay drawn with the
/// units (without bars), rising 40 over 0.14 s above their own tiles; then each takes the
/// other's tile and flies there in the flight layer (0.18 s per 126 units; the lower id
/// on top, see bb-render), and lands (50 over 0.23 s), calling `onDone(unit, tag)`. The speed divides every time. Ours
/// used to leave both undrawn while they rose: they blinked out for 0.14 s.
#[test]
fn a_switch_in_view_keeps_both_units_drawn_as_they_rise() {
	let Some(engine) = timed_battle() else { return };
	let pair = player::eval_string(&engine, PAIR).unwrap().unwrap();
	let ids: Vec<i64> = pair.split(' ').map(|x| x.parse().expect("two neighbouring brothers")).collect();
	let start = |i: usize| numbers(&player::eval_string(&engine, &format!("::__tiles[{i}].Pos.X + \" \" + ::__tiles[{i}].Pos.Y")).unwrap().unwrap());
	let from = [start(0), start(1)];
	engine
		.run_script(
			"::Tactical.State.m.IsAIPaused = true; ::__done <- []; ::__t0 <- ::Time.getVirtualTimeF();
			::__answer <- ::Tactical.getNavigator().switchEntities(::__pair[0], ::__pair[1], function ( _e, _tag ) {
				::__done.push(_e.getID() + \" \" + _tag + \" \" + (::Time.getVirtualTimeF() - ::__t0));
			}, \"rotation\", 1.0);",
			"switch",
		)
		.unwrap();
	assert_eq!(player::eval_i64(&engine, "::__answer ? 1 : 0").unwrap(), Some(1), "it switches");
	let s = numbers(&player::eval_string(&engine, PAIR_SAMPLE).unwrap().unwrap());
	assert_eq!(s[..8], [0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0], "both off the map, travelling, at (0, 0) until the next update");
	let t0 = s[8];

	// Per unit: when it took off into the flight layer, and when it began landing.
	let mut flew: [Option<f64>; 2] = [None, None];
	let mut landing: [Option<f64>; 2] = [None, None];
	for frame in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
		let s = numbers(&player::eval_string(&engine, PAIR_SAMPLE).unwrap().unwrap());
		let t = s[8] - t0;
		let scene = engine.tactical_scene();
		let mut travelling = false;
		for i in 0..2 {
			let (placed, moving, x, y) = (s[i * 4] == 1.0, s[i * 4 + 1] == 1.0, s[i * 4 + 2], s[i * 4 + 3]);
			travelling |= moving;
			let drawn = scene.entities.iter().find(|e| e.id == ids[i]).unwrap_or_else(|| panic!("frame {frame} ({t:.3} s): unit {i} isn't drawn"));
			let (own, other) = (&from[i], &from[1 - i]);
			if !placed {
				// Rising over its own tile, among the units.
				assert!(flew[i].is_none(), "off the map again after taking off");
				assert!(drawn.off_map && !drawn.flying, "frame {frame}: rising off the map, drawn with the units");
				assert_eq!(x, own[0], "straight up");
				assert!(y > own[1] && y <= own[1] + 40.0 + 1e-3, "at most 40 up ({})", y - own[1]);
				assert!(drawn.overlay.is_some(), "its bars kept (the renderer leaves them out off the map)");
			} else if drawn.flying {
				flew[i].get_or_insert(t);
				assert!(landing[i].is_none(), "flying again after landing");
			} else if moving {
				landing[i].get_or_insert(t);
				assert_eq!(x, other[0], "dropping straight down onto the other's tile");
				assert!(y >= other[1] - 1e-3 && y <= other[1] + 50.0 + 1e-3, "at most 50 up ({})", y - other[1]);
			}
		}
		if !travelling {
			break;
		}
	}
	// Lift-off is 0.14 s, from the first update: it ends at the first frame at or after.
	for i in 0..2 {
		let flew = flew[i].unwrap_or_else(|| panic!("unit {i} never flew"));
		assert!((0.14..0.14 + 0.034).contains(&flew), "unit {i} took off at {flew:.3} s");
		let across = ((from[0][0] - from[1][0]).powi(2) + (from[1 - i][1] + 50.0 - from[i][1] - 40.0).powi(2)).sqrt();
		let flight = (across / 126.0 * 0.18) as f64;
		let landed = landing[i].unwrap_or_else(|| panic!("unit {i} never landed"));
		assert!(landed >= flew + flight - 1e-3 && landed < flew + flight + 0.034, "unit {i}: {flight:.3} s of flight, landing at {landed:.3} s");
	}
	let done = player::eval_string(&engine, "::__done.reduce(@(a, b) a + \"; \" + b)").unwrap().unwrap();
	assert_eq!(done.split("; ").count(), 2, "onDone(unit, tag) once per landing: {done}");
	for (i, id) in ids.iter().enumerate() {
		let line = done.split("; ").find(|l| l.starts_with(&format!("{id} rotation "))).unwrap_or_else(|| panic!("unit {i}'s onDone: {done}"));
		let at: f64 = line.rsplit(' ').next().unwrap().parse().unwrap();
		let landed = landing[i].unwrap();
		assert!(at >= landed + 0.23 - 1e-3 && at < landed + 0.23 + 0.034, "unit {i} landed for 0.23 s: done at {at:.3} s");
		let tile = player::eval_string(&engine, &format!("::__pair[{i}].getTile().Pos.X + \" \" + ::__pair[{i}].getTile().Pos.Y")).unwrap().unwrap();
		assert_eq!(numbers(&tile), from[1 - i], "unit {i} on the other's tile");
	}
}
