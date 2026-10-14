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

use bb_engine::{harness, tactical_player as player};
use squirrel::SqTable;
use squirrel_bb::KeyInput;

/// The engine's Escape key code.
const KEY_ESCAPE: i64 = 41;

fn frame(engine: &harness::GameEngine) {
	if let Err(e) = harness::run_frame(engine, 1000.0 / 60.0) {
		let diag = player::eval_string(engine, r#"(function() {
			local s = "";
			foreach (b in ::World.getPlayerRoster().getAll()) {
				foreach (it in b.getItems().getAllItems()) {
					local c = it.getContainer();
					local dead = c == null ? "no container" : (c.m.Actor == null ? "null actor" : (c.m.Actor.isNull() ? "DEAD actor" : (c.m.Actor.get() == b ? "ok" : "OTHER actor " + c.m.Actor.getName())));
					if (dead != "ok") s += b.getName() + ": " + it.getID() + " (" + it.ClassName + ") " + dead + (c != null && c == b.getItems() ? " [own container]" : " [other container]") + "; ";
				}
			}
			return s;
		})()"#).ok().flatten().unwrap_or_default();
		panic!("frame: {e}\nitems whose actor is wrong: {diag}");
	}
}

/// Run frames until `cond` (a Squirrel expression) holds; panics after `max`.
fn until(engine: &harness::GameEngine, max: u32, what: &str, cond: &str) {
	for _ in 0..max {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap_or_else(|e| panic!("{what}: {e}")) == Some(1) {
			return;
		}
		frame(engine);
	}
	panic!("{what}: still not true after {max} frames: {cond}");
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
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	smoke(harness::open_install(dir).expect("open install"));
}

/// The same, with the install's mods mounted (32 of them, among them Modern
/// Hooks, MSU, Reforged and Modular Vanilla, which change the battles). Skips without
/// Modern Hooks.
#[test]
fn the_installed_mods_play_through_menu_campaign_battle_save_and_load() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	let Ok(mods) = harness::mod_archives(dir) else {
		eprintln!("skipping: {} not present", dir.display());
		return;
	};
	if !mods.iter().any(|m| m.file_name().is_some_and(|n| n.to_string_lossy().starts_with("mod_modern_hooks"))) {
		eprintln!("skipping: Modern Hooks isn't installed");
		return;
	}
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

	// Let the world run (the intro event is answered), then go after the nearest hostile
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
		local player = ::World.State.getPlayer();
		local target = null, best = 0;
		foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 100000.0))
		{
			if (!e.isParty() || e.getID() == player.getID() || e.isAlliedWithPlayer() || !e.isAttackable() || e.getTroops().len() == 0) continue;
			local d = e.getDistanceTo(player);
			if (target == null || d < best) { target = e; best = d; }
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
		::__autoplayEngage = true;
		::__autoplayFight = true;
		// Each second: march toward it as a click on the ground does (a path from the
		// world's navigator), and once it's in sight, attack it as a click on it does.
		::__smokeChase <- function()
		{
			local target = ::__smokeTarget;
			if (target.isNull() || !target.isAlive()) throw "the target is gone";
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
	let root = engine.root_state();
	root.key_input(engine.vm(), KeyInput::pressed(KEY_ESCAPE)).expect("escape");
	root.key_input(engine.vm(), KeyInput::released(KEY_ESCAPE)).expect("escape");
	until(&engine, 60, "the world menu", &format!("{WORLD_MENU}.isVisible() && !{WORLD_MENU}.isAnimating()"));
	script(&engine, "save menu", &format!("{WORLD_MENU}.getMainMenuModule().onSaveCampaignButtonPressed();"));
	frame(&engine);
	script(&engine, "save", &format!("{WORLD_MENU}.getSaveCampaignMenuModule().onSaveButtonPressed(\"smoke\");"));
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
	assert!(listed.contains("smoke/"), "the save isn't listed: {listed}");
	script(&engine, "load", &format!("{MENU}.getLoadCampaignMenuModule().onLoadButtonPressed(\"smoke\");"));
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
	for _ in 0..1200 {
		frame(&engine);
	}
	let later = player::eval_i64(&engine, "::World.getTime().Time.tointeger()").unwrap().unwrap();
	assert!(later > start, "the loaded world's clock doesn't run ({start} → {later})");
}
