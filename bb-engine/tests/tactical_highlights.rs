//! What a battle highlights for the player, as the battle's scene exports it: the path's
//! footsteps when a brother's move is previewed (one per step past his tile, pointing the
//! way the path goes, pale while he can afford it and grey past that), a skill's range and
//! targets, the icons over tiles, which fade out when cleared, and, with the setting, the
//! marks on the tiles map objects block. All of it goes when the scripts clear it.
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

/// An empty tile `n` steps from the acting unit in one direction, on its level, through
/// tiles the player has discovered: "x y".
fn straight(n: u32) -> String {
	format!(
		r#"(function() {{
	local e = ::Tactical.TurnSequenceBar.getActiveEntity(), my = e.getTile();
	for (local d = 0; d < 6; d++)
	{{
		local t = my, ok = true;
		for (local i = 0; i < {n} && ok; i++)
		{{
			if (!t.hasNextTile(d)) {{ ok = false; break; }}
			t = t.getNextTile(d);
			ok = t.IsEmpty && t.Level == my.Level && t.IsDiscovered;
		}}
		if (ok) return t.SquareCoords.X + " " + t.SquareCoords.Y;
	}}
	return "";
}})()"#
	)
}

/// Discovered empty tiles 6 away from the acting unit on its level: "x y;x y;...".
const FAR: &str = r#"(function() {
	local e = ::Tactical.TurnSequenceBar.getActiveEntity(), my = e.getTile(), out = "";
	for (local x = my.SquareCoords.X - 8; x <= my.SquareCoords.X + 8; x++)
		for (local y = my.SquareCoords.Y - 8; y <= my.SquareCoords.Y + 8; y++)
		{
			if (!::Tactical.isValidTileSquare(x, y)) continue;
			local t = ::Tactical.getTileSquare(x, y);
			if (t.IsDiscovered && t.IsEmpty && t.Level == my.Level && my.getDistanceTo(t) == 6) out += x + " " + y + ";";
		}
	return out;
})()"#;

fn xy(s: &str) -> (i64, i64) {
	let v: Vec<i64> = s.split(' ').map(|x| x.parse().unwrap()).collect();
	(v[0], v[1])
}

#[test]
fn the_battle_highlights_what_the_player_is_about_to_do() {
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

	// A move two tiles off, previewed: two pale footsteps, on the tiles, pointing one way.
	let (x, y) = xy(&player::eval_string(&engine, &straight(2)).unwrap().unwrap());
	player::click_tile(&engine, x, y).unwrap();
	let path = engine.tactical_scene().path;
	assert_eq!(path.len(), 2, "a footstep per step: {path:?}");
	assert!(path.iter().all(|s| s.color == 0xffe0_e5a2), "affordable: {path:?}");
	assert_eq!(path[0].brush, path[1].brush, "straight on");
	assert!(path[0].brush.starts_with("steps_"));
	let dest = player::eval_string(&engine, &format!("::Tactical.getTileSquare({x}, {y}).Pos.X + \" \" + ::Tactical.getTileSquare({x}, {y}).Pos.Y")).unwrap().unwrap();
	let dest: Vec<f64> = dest.split(' ').map(|v| v.parse().unwrap()).collect();
	assert_eq!(path[1].pos, (dest[0], dest[1]), "on the destination");

	// A move further than the brother's action points take him (6 tiles, at 2 or more
	// each, of his 9): grey past them, from where they run out on.
	let far = player::eval_string(&engine, FAR).unwrap().unwrap();
	let mut previewed = 0;
	for target in far.split(';').filter(|t| !t.is_empty()) {
		let (x, y) = xy(target);
		player::click_tile(&engine, x, y).unwrap();
		let path = engine.tactical_scene().path;
		if path.len() < 6 || path.len() > 8 {
			continue;
		}
		assert_eq!(path[0].color, 0xffe0_e5a2, "the first steps are affordable: {path:?}");
		let grey = path.iter().position(|s| s.color == 0xff77_7777).expect("some steps unaffordable");
		assert!(path[grey..].iter().all(|s| s.color == 0xff77_7777), "grey from there on: {path:?}");
		previewed += 1;
		break;
	}
	assert_eq!(previewed, 1, "a long move previewed");
	engine.run_script("::Tactical.State.cancelEntityPath(::Tactical.TurnSequenceBar.getActiveEntity());", "cancel").unwrap();
	assert!(engine.tactical_scene().path.is_empty(), "cleared");

	// A skill's range: its tiles in range, some of them targets; cleared, none.
	engine
		.run_script(
			"local e = ::Tactical.TurnSequenceBar.getActiveEntity(); \
			 local sk = null; foreach (s in e.getSkills().queryActives()) if (s.isAttack() && s.isTargeted()) { sk = s; break; } \
			 ::__range <- sk.getMaxRange(); \
			 ::Tactical.getHighlighter().clear(); ::Tactical.getHighlighter().highlightRangeOfSkill(sk, e);",
			"range",
		)
		.unwrap();
	let scene = engine.tactical_scene();
	let lit: Vec<&str> = scene.tiles.iter().filter_map(|t| t.highlight.as_ref().map(|(b, _)| b.as_str())).collect();
	assert!(!lit.is_empty(), "the skill's range shows");
	assert!(lit.iter().all(|b| *b == "zone_range_overlay" || *b == "zone_target_overlay"), "{lit:?}");
	engine.run_script("::Tactical.getHighlighter().clear();", "clear").unwrap();
	assert!(engine.tactical_scene().tiles.iter().all(|t| t.highlight.is_none()), "cleared");

	// An icon over a tile: white where the script puts it; cleared, it fades and goes.
	engine
		.run_script(
			"local t = ::Tactical.TurnSequenceBar.getActiveEntity().getTile(); \
			 ::Tactical.getHighlighter().addOverlayIcon(::Const.Tactical.Settings.AreaOfEffectIcon, t, t.Pos.X, t.Pos.Y);",
			"icon",
		)
		.unwrap();
	let icons = engine.tactical_scene().overlay_icons;
	assert_eq!(icons.len(), 1);
	assert_eq!((icons[0].brush.as_str(), icons[0].color), ("overlay_aoe_attack", 0xffff_ffff));
	engine.run_script("::Tactical.getHighlighter().clearOverlayIcons();", "clear icons").unwrap();
	for _ in 0..20 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	assert!(engine.tactical_scene().overlay_icons.is_empty(), "faded out and gone");

	// The blocked tiles: a mark where a map object stands, while the setting shows them.
	engine.run_script("::Tactical.setBlockedTileHighlightsVisibility(true);", "show").unwrap();
	let scene = engine.tactical_scene();
	let mut objects: Vec<(i64, i64)> = scene.entities.iter().filter(|e| !e.is_actor && e.tile.is_some()).map(|e| (e.pos.0 as i64, e.pos.1 as i64)).collect();
	let mut marks: Vec<(i64, i64)> = scene.blocked_tiles.iter().map(|m| (m.pos.0 as i64, m.pos.1 as i64)).collect();
	objects.sort();
	marks.sort();
	// Made once, at the battle's start: objects placed since have none.
	assert!(!marks.is_empty(), "the map has objects");
	assert!(marks.iter().all(|m| objects.contains(m)), "marks only where objects stand: {marks:?} vs {objects:?}");
	assert!(scene.blocked_tiles.iter().all(|m| m.brush == "zone_target_overlay" && m.color == 0xa8ff_ffff));
	engine.run_script("::Tactical.setBlockedTileHighlightsVisibility(false);", "hide").unwrap();
	assert!(engine.tactical_scene().blocked_tiles.is_empty(), "hidden");
}
