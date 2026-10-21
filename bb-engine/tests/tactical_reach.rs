//! How far a brother can move, as a battle shows it: hovering a move (the first click
//! on a tile) previews its cost on the turn bar's action point and fatigue bars
//! (`setActiveEntityCostsPreview`, from `getCostForPath`), and the path's footsteps are
//! pale for the steps he can afford and grey past them. The preview reaches the page
//! (`updateCostsPreview`) with what the move leaves him.
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

/// A discovered empty tile `n` steps from the acting unit in one direction, on its
/// level: "x y".
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

/// What the move to the clicked tile costs the acting unit, as the tactical state
/// computes it for the preview, and what the turn bar keeps as its preview:
/// "ap fatigue apCost fatigueCost tiles previewAp previewFatigue".
const PREVIEW: &str = r#"(function() {
	local e = ::Tactical.TurnSequenceBar.getActiveEntity(), nav = ::Tactical.getNavigator();
	local c = nav.getCostForPath(e, nav.getLastSettings(), e.getActionPoints(), e.getFatigueMax() - e.getFatigue());
	local p = ::Tactical.TurnSequenceBar.m.ActiveEntityCostsPreview;
	return e.getActionPoints() + " " + e.getFatigue() + " " + c.ActionPoints + " " + c.Fatigue + " " + c.Tiles
		+ " " + (p == null ? -1 : p.actionPointsPreview) + " " + (p == null ? -1 : p.fatiguePreview);
})()"#;

fn numbers(s: &str) -> Vec<i64> {
	s.split(' ').map(|x| x.parse().unwrap()).collect()
}

#[test]
fn a_brothers_reach_shows_on_the_turn_bar_and_the_path() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
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

	for n in [2, 6] {
		let target = player::eval_string(&engine, &straight(n)).unwrap().unwrap();
		if target.is_empty() {
			continue;
		}
		let t: Vec<i64> = numbers(&target);
		let sent_before = engine.ui().calls().iter().filter(|c| c.method == "updateCostsPreview").count();
		player::click_tile(&engine, t[0], t[1]).unwrap();
		let v = numbers(&player::eval_string(&engine, PREVIEW).unwrap().unwrap());
		let (ap, fatigue, ap_cost, fatigue_cost, tiles, preview_ap, preview_fatigue) = (v[0], v[1], v[2], v[3], v[4], v[5], v[6]);
		eprintln!("{n} tiles off: AP {ap} - {ap_cost}, fatigue {fatigue} + {fatigue_cost}, {tiles} affordable; preview {preview_ap} AP, {preview_fatigue} fatigue");
		assert!(tiles > 0 && ap_cost > 0, "a move he can at least begin");
		assert_eq!(preview_ap, ap - ap_cost, "the AP bar previews what the move leaves");
		assert_eq!(preview_fatigue, fatigue + fatigue_cost, "the fatigue bar previews what the move adds");
		// The page gets it.
		let sent: Vec<_> = engine.ui().calls().into_iter().filter(|c| c.method == "updateCostsPreview").collect();
		assert!(sent.len() > sent_before, "updateCostsPreview reaches the page");
		let data = &sent.last().unwrap().data;
		let field = |k: &str| data.get_member::<i64>(engine.vm(), k).unwrap_or(-1);
		assert_eq!((field("actionPointsPreview"), field("fatiguePreview")), (preview_ap, preview_fatigue), "with the preview's values");
		// The footsteps: pale as far as the AP and fatigue go, grey past them.
		let path = engine.tactical_scene().path;
		let pale = path.iter().take_while(|s| s.color == 0xffe0_e5a2).count() as i64;
		assert_eq!(pale, tiles, "pale footsteps for the affordable steps: {path:?}");
	}
}
