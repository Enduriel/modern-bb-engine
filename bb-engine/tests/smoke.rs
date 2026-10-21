//! The whole game headless, through its own menus as a player goes: the main menu's
//! New Campaign page starts a campaign (the settings its page sends), the world runs, a
//! battle is engaged and fought out (the AI plays both sides), and back on the world map
//! the campaign is saved from the world menu (Escape, Save Campaign), the game quits to
//! the main menu and loads that save from Load Campaign. The loaded world must read as
//! it did when saved, the battle's outcome included, and keep running.
//!
//! Menu buttons are the screens' JS→SQ callbacks, keys go through the states' key input.
//! Saves stay in memory (the engine's default store).
//!
//! Skips if no Steam install. Slow in debug builds: it runs all of worldgen first.

use std::path::Path;

use bb_engine::mod_corpus::{self, CorpusError};
use bb_engine::{harness, tactical_player as player};
use squirrel::SqTable;
use squirrel_bb::KeyInput;

/// The engine's Escape key code.
const KEY_ESCAPE: i64 = 41;

fn frame(engine: &harness::GameEngine) {
	harness::run_frame(engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame: {e}"));
}

/// Run frames until `cond` (a Squirrel expression) holds; panics after `max`.
fn until(engine: &harness::GameEngine, max: u32, what: &str, cond: &str) {
	for _ in 0..max {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap_or_else(|e| panic!("{what}: {e}")) == Some(1) {
			return;
		}
		frame(engine);
	}
	panic!("{what}: still not true after {max} frames: {cond}\nscreen: {}", screen_summary(engine));
}

/// What the world state has up, for a failure message.
fn screen_summary(engine: &harness::GameEngine) -> String {
	player::eval_string(
		engine,
		r#"(function() {
			if (!("State" in ::World) || ::World.State == null) return "no world";
			local ws = ::World.State, s = "paused " + ws.isPaused() + ", menu stack " + ws.m.MenuStack.hasBacksteps();
			foreach (name, v in ws.m)
				if (v != null && typeof v == "table" && "isVisible" in v && name.find("Screen") != null)
					try { if (v.isVisible()) s += ", " + name + " visible"; } catch (e) {}
			if ("MSU" in ::getroottable()) s += ", MSU popup " + ::MSU.Popup.isVisible();
			return s;
		})()"#,
	)
	.ok()
	.flatten()
	.unwrap_or_default()
}

fn script(engine: &harness::GameEngine, what: &str, src: &str) {
	engine.run_script(src, what).unwrap_or_else(|e| panic!("{what}: {e}"));
}

/// The main menu's screen, from the main menu state.
const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";

/// Unpause the world (Space), unless a screen or a battle has it stopped.
const UNPAUSE: &str = "if (::World.State.isPaused() && !::World.State.m.EventScreen.isVisible()) { ::World.State.setAutoPause(false); ::World.State.setPause(false); }";

/// The world state's menu (Escape on the world map).
const WORLD_MENU: &str = "::World.State.m.WorldMenuScreen";

/// What must survive the save and load: the clock, money, the company (with what the
/// battle did to it) and the stash.
const SUMMARY: &str = r#"
(function() {
	local t = ::World.getTime();
	local s = "day=" + t.Days + " money=" + ::World.Assets.getMoney();
	foreach (b in ::World.getPlayerRoster().getAll())
		s += " " + b.getName() + ":" + b.getHitpoints() + "/" + b.getHitpointsMax() + ":xp" + b.getXP()
			+ ":inj" + b.getSkills().query(::Const.SkillType.Injury).len() + ":items" + b.getItems().getAllItems().len();
	local items = [];
	foreach (it in ::World.Assets.getStash().getItems()) if (it != null) items.push(it.getID());
	items.sort();
	s += " stash=" + items.len();
	foreach (i in items) s += "," + i;
	return s;
})()
"#;

#[test]
fn menu_to_campaign_to_battle_to_save_and_load() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
		return;
	}
	smoke(harness::open_install(dir).expect("open install"));
}

/// The same, with the install's mods mounted (32 of them, among them Modern
/// Hooks, MSU, Reforged and Modular Vanilla, which change the battles; the pinned corpus,
/// `bb_engine::mod_corpus`). Skips without mods; fails if they are not the corpus.
#[test]
fn the_installed_mods_play_through_menu_campaign_battle_save_and_load() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	let mods = match mod_corpus::installed(dir) {
		Ok(mods) => mods,
		Err(CorpusError::Absent(why)) => return bb_engine::harness::skip(why),
		Err(e) => panic!("{e}"),
	};
	let mut archives = harness::data_archives(dir).expect("list the data archives");
	archives.extend(mods);
	smoke(harness::open_mounted(dir, archives, |_| {}).expect("open the archives"));
}

fn smoke(mut engine: harness::GameEngine) {
	// A script error prints its call stack (shown if the test fails).
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	harness::install_autoplay(&engine).expect("autoplay");

	// Main menu → New Campaign → Start, with the settings the page sends.
	until(&engine, 60, "main menu", &format!("{MENU}.isVisible() && !{MENU}.isAnimating()"));
	script(&engine, "new campaign", &format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"));
	frame(&engine);
	script(
		&engine,
		"start",
		&format!(
			"{MENU}.getNewCampaignMenuModule().onStartButtonPressed({{
				companyName = \"Smoke Test Company\", banner = \"banner_04\",
				combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
				ironman = false, explorationDifficulty = false,
				lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
				seed = \"rust-engine-test\", origin = \"scenario.early_access\"
			}});"
		),
	);
	// MSU (if installed) asks for its new-campaign settings first: keep the defaults.
	for _ in 0..600 {
		let started = "(\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";
		if player::eval_i64(&engine, &format!("({started}) ? 1 : 0")).unwrap() == Some(1) {
			break;
		}
		if player::eval_i64(&engine, "((\"MSU\" in ::getroottable()) && ::MSU.SettingsScreen.isVisible()) ? 1 : 0").unwrap() == Some(1) {
			script(&engine, "MSU settings", "::MSU.SettingsScreen.onSaveButtonPressed({});");
		}
		frame(&engine);
	}
	until(
		&engine,
		1,
		"the campaign starts",
		"(\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()",
	);

	// Let the world run (the intro event is answered), then go after the weakest hostile
	// party the player can see. A click on it sets the world state's AutoAttack; the
	// world camera isn't real yet (no position for screenToWorld), so the test sets
	// that directly and lets the company march and engage.
	// A player unpauses whenever the game pauses itself (with Configurable Pause it does
	// so after every event, the intro's included).
	for _ in 0..10 {
		script(&engine, "unpause", UNPAUSE);
		for _ in 0..60 {
			frame(&engine);
		}
	}
	script(
		&engine,
		"target",
		r#"
		// The weakest hostile party (a lost company ends the game), the nearest of equals.
		::__smokePickTarget <- function()
		{
			local player = ::World.State.getPlayer();
			local target = null, best = 0, weakest = 0;
			foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 100000.0))
			{
				if (!e.isParty() || e.getID() == player.getID() || e.isAlliedWithPlayer() || !e.isAttackable() || e.getTroops().len() == 0) continue;
				local d = e.getDistanceTo(player), strength = e.getStrength();
				if (target == null || strength < weakest || (strength == weakest && d < best)) { target = e; best = d; weakest = strength; }
			}
			if (target == null)
			{
				local parties = 0, hostile = 0;
				foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 100000.0))
					if (e.isParty() && e.getID() != player.getID()) { parties++; if (!e.isAlliedWithPlayer()) hostile++; }
				throw "no hostile party (day " + ::World.getTime().Days + ", time " + ::World.getTime().Time + ", paused "
					+ ::World.State.isPaused() + ", " + parties + " parties, " + hostile + " not allied, event "
					+ ::World.State.m.EventScreen.isVisible() + (("MSU" in ::getroottable()) ? ", MSU popup " + ::MSU.Popup.isVisible() : "")
					+ ", menu stack " + ::World.State.m.MenuStack.hasBacksteps() + ")";
			}
			::logInfo("smoke: attacking " + target.getName() + " at distance " + best + (target.isHiddenToPlayer() ? " (hidden)" : ""));
			::__smokeTarget <- ::WeakTableRef(target);
		};
		::__smokePickTarget();
		// Sturdier brothers: the test is about the game's flow, and a company that loses
		// the battle ends the game (harder with mods such as Reforged).
		foreach (bro in ::World.getPlayerRoster().getAll())
		{
			local b = bro.getBaseProperties();
			b.Hitpoints += 500;
			b.MeleeDefense += 40;
			b.RangedDefense += 40;
			bro.getSkills().update();
			bro.setHitpoints(bro.getHitpointsMax());
		}
		::__autoplayEngage = true;
		::__autoplayFight = true;
		// A brother who falls is a brother lost, as when a player fights the battle.
		::__autoplayDieAsPlayers = true;
		// Each second: march toward it as a click on the ground does (a path from the
		// world's navigator), and once it's in sight, attack it as a click on it does.
		::__smokeChase <- function()
		{
			// One that's gone (destroyed by someone else, or home) is replaced.
			if (::__smokeTarget.isNull() || !::__smokeTarget.isAlive()) ::__smokePickTarget();
			local target = ::__smokeTarget;
			local ws = ::World.State;
			if (ws.m.AutoAttack != null) return;
			if (!target.isHiddenToPlayer()) { ws.m.AutoAttack = ::WeakTableRef(target.get()); return; }
			local nav = ::World.getNavigator().createSettings();
			nav.ActionPointCosts = ::Const.World.TerrainTypeNavCost;
			nav.RoadMult = 1.0 / ::Const.World.MovementSettings.RoadMult;
			local path = ::World.getNavigator().findPath(ws.getPlayer().getTile(), target.getTile(), nav, 0);
			if (!path.isEmpty()) ws.getPlayer().setPath(path);
		};
		"#,
	);
	for second in 0..600 {
		if player::eval_i64(&engine, "(\"State\" in ::Tactical) && ::Tactical.State != null ? 1 : 0").unwrap() == Some(1) {
			break;
		}
		assert!(second < 599, "no battle after ten minutes of marching");
		script(&engine, "unpause", UNPAUSE);
		script(&engine, "chase", "::__smokeChase();");
		for _ in 0..60 {
			frame(&engine);
		}
	}
	until(&engine, 60_000, "the battle ends", "::RootState.get(\"TacticalFromWorldState\") == null && (\"__lastCombatResult\" in ::getroottable())");
	let result = SqTable::root(engine.vm()).get::<i64>("__lastCombatResult");
	assert!(matches!(result, Some(r) if r != 0), "no combat result: {result:?}");
	script(&engine, "no more battles", "::__autoplayEngage = false; ::__autoplayFight = false;");
	until(&engine, 600, "back on the world map", "::World.State.isVisible() && !::LoadingScreen.isVisible()");

	// Escape opens the world menu (no battle is on offer: world parties stand still
	// while a battle runs, and the ones nearby are stunned after it); Save Campaign,
	// under a name.
	// Escape first cancels whatever the company was still doing (a march, an attack), as
	// in the game; a player presses it again.
	let root = engine.root_state();
	let menu_up = format!("{WORLD_MENU}.isVisible() && !{WORLD_MENU}.isAnimating()");
	for _ in 0..3 {
		root.key_input(engine.vm(), KeyInput::pressed(KEY_ESCAPE)).expect("escape");
		root.key_input(engine.vm(), KeyInput::released(KEY_ESCAPE)).expect("escape");
		for _ in 0..60 {
			frame(&engine);
		}
		if player::eval_i64(&engine, &format!("({menu_up}) ? 1 : 0")).unwrap() == Some(1) {
			break;
		}
	}
	until(&engine, 1, "the world menu", &menu_up);
	script(&engine, "save menu", &format!("{WORLD_MENU}.getMainMenuModule().onSaveCampaignButtonPressed();"));
	frame(&engine);
	script(&engine, "save", &format!("{WORLD_MENU}.getSaveCampaignMenuModule().onSaveButtonPressed(\"bb_probe_smoke\");"));
	let saved = player::eval_string(&engine, SUMMARY).unwrap().unwrap();
	frame(&engine);

	// Quit to the main menu.
	script(&engine, "quit", &format!("{WORLD_MENU}.getMainMenuModule().onQuitButtonPressed();"));
	until(
		&engine,
		600,
		"back at the main menu",
		&format!("::RootState.get(\"WorldState\") == null && {MENU}.isVisible() && !{MENU}.isAnimating()"),
	);

	// Load Campaign: the save is listed; load it.
	script(&engine, "load menu", &format!("{MENU}.getMainMenuModule().onLoadCampaignButtonPressed();"));
	frame(&engine);
	let listed = player::eval_string(
		&engine,
		"(function() { local s = \"\"; foreach (c in ::UIDataHelper.convertCampaignStoragesToUIData()) s += c.fileName + \"/\" + c.name + \" \"; return s; })()",
	)
	.unwrap()
	.unwrap();
	assert!(listed.contains("bb_probe_smoke/"), "the save isn't listed: {listed}");
	script(&engine, "load", &format!("{MENU}.getLoadCampaignMenuModule().onLoadButtonPressed(\"bb_probe_smoke\");"));
	until(
		&engine,
		600,
		"the campaign loads",
		"::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()",
	);
	let loaded = player::eval_string(&engine, SUMMARY).unwrap().unwrap();
	assert_eq!(saved, loaded, "the loaded campaign differs from the saved one");

	// And it keeps running.
	script(&engine, "unpause", "::World.State.setAutoPause(false); ::World.State.setPause(false);");
	let start = player::eval_i64(&engine, "::World.getTime().Time.tointeger()").unwrap().unwrap();
	for i in 0..1200 {
		// An event (answered by the scripted player) or a mod (Configurable Pause) may
		// pause the world meanwhile, as everywhere else in this test.
		if i % 60 == 0 {
			script(&engine, "unpause", UNPAUSE);
		}
		frame(&engine);
	}
	let later = player::eval_i64(&engine, "::World.getTime().Time.tointeger()").unwrap().unwrap();
	assert!(later > start, "the loaded world's clock doesn't run ({start} → {later})");
}
