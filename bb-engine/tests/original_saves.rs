//! The original's saves: a campaign the original saved (vanilla with Modern Hooks,
//! right after it started; see `harness::ORIGINAL_SAVES_DIR`) loads from the main menu's
//! Load Campaign, reads as the original had it (its own dump of the state, taken by the
//! same script), and keeps running.
//!
//! Skips without the Steam install or the saves.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

/// The state the original's capture probe dumps (its `dump`), one line per fact.
const DUMP: &str = r#"
(function() {
	local out = [];
	local t = ::World.getTime();
	out.push("time virtual=" + ::Time.getVirtualTimeF() + " days=" + t.Days + " hours=" + t.Hours + " minutes=" + t.Minutes + " daytime=" + t.IsDaytime);
	local cam = ::World.getCamera();
	out.push("camera pos=" + cam.getPos().X + "," + cam.getPos().Y + " zoom=" + cam.Zoom);
	local a = ::World.Assets;
	out.push("assets money=" + a.getMoney() + " food=" + a.getFood() + " ammo=" + a.getAmmo() + " medicine=" + a.getMedicine() + " tools=" + a.getArmorParts() + " business=" + a.getBusinessReputation() + " moral=" + a.getMoralReputation());
	local p = ::World.State.getPlayer();
	out.push("player id=" + p.getID() + " pos=" + p.getPos().X + "," + p.getPos().Y + " tile=" + p.getTile().SquareCoords.X + "," + p.getTile().SquareCoords.Y);
	local rows = [];
	foreach (e in ::World.getAllEntitiesAtPos(::createVec(0, 0), 10000000.0))
	{
		local cls = ("ClassName" in e) ? e.ClassName : "?";
		rows.push([e.getID(), e.getID() + "|" + cls + "|" + e.getName() + "|" + e.getPos().X + "|" + e.getPos().Y + "|" + e.getFaction() + "|" + e.isVisibleToEntity(p, 10000)]);
	}
	rows.sort(@(x, y) x[0] <=> y[0]);
	out.push("entities " + rows.len());
	foreach (r in rows) out.push("E " + r[1]);
	foreach (b in ::World.getPlayerRoster().getAll())
		out.push("R " + b.getName() + "|" + b.getHitpoints() + "|" + b.getXP() + "|" + b.getLevel() + "|" + b.getBackground().getID() + "|" + b.getItems().getAllItems().len());
	out.push("stash " + ::World.Assets.getStash().getItems().len() + " factions " + ::World.FactionManager.getFactions().len() + " contracts " + ::World.Contracts.getOpenContracts().len());
	local s = "";
	foreach (l in out) s += l + "\n";
	return s;
})()
"#;

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
	panic!("{what}: still not true after {max} frames: {cond}");
}

fn script(engine: &harness::GameEngine, what: &str, src: &str) {
	engine.run_script(src, what).unwrap_or_else(|e| panic!("{what}: {e}"));
}

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";

/// The fixtures, copied where the engine may write (the game saves on some actions).
fn saves_dir(test: &str) -> Option<std::path::PathBuf> {
	let from = Path::new(harness::ORIGINAL_SAVES_DIR);
	if !Path::new(harness::STEAM_DATA_DIR).exists() || !from.join("new_campaign.sav").exists() {
		eprintln!("skipping: needs the Steam install and {}", from.display());
		return None;
	}
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	for f in std::fs::read_dir(from).unwrap().flatten() {
		std::fs::copy(f.path(), dir.join(f.file_name())).unwrap();
	}
	Some(dir)
}

/// An engine at the main menu with saves in `dir`, where it loads campaign `name`.
fn load(dir: &Path, name: &str) -> harness::GameEngine {
	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open the install");
	// A script error prints its call stack (shown if the test fails).
	engine.vm_mut().install_standard_error_handlers();
	engine.saves().set_directory(dir.to_path_buf());
	harness::boot_menu(&engine).expect("boot the menu");
	until(&engine, 60, "main menu", &format!("{MENU}.isVisible() && !{MENU}.isAnimating()"));
	// The world state ignores a load within 4 s (real time) of the game's start, which a
	// player never gets to.
	until(&engine, 600, "4 s at the menu", "::Time.getRealTimeF() > 4.5");
	script(&engine, "load menu", &format!("{MENU}.getMainMenuModule().onLoadCampaignButtonPressed();"));
	frame(&engine);
	script(&engine, "load", &format!("{MENU}.getLoadCampaignMenuModule().onLoadButtonPressed(\"{name}\");"));
	until(
		&engine,
		600,
		"the campaign loads",
		"::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()",
	);
	engine
}

/// Load `name` and compare the world with the original's dump of it.
fn loads_as_the_original_had_it(test: &str, name: &str) -> Option<harness::GameEngine> {
	let dir = saves_dir(test)?;
	let engine = load(&dir, name);
	let ours = player::eval_string(&engine, DUMP).unwrap().unwrap();
	let original = std::fs::read_to_string(dir.join(format!("{name}.dump.txt"))).unwrap();
	let (ours, original): (Vec<&str>, Vec<&str>) = (ours.lines().collect(), original.lines().collect());
	let differ: Vec<String> = ours
		.iter()
		.zip(&original)
		.filter(|(a, b)| a != b)
		.take(20)
		.map(|(a, b)| format!("  ours     {a}\n  original {b}"))
		.collect();
	assert!(
		differ.is_empty() && ours.len() == original.len(),
		"{name}: the loaded world differs from the original's ({} vs {} lines):\n{}",
		ours.len(),
		original.len(),
		differ.join("\n")
	);
	Some(engine)
}

#[test]
fn an_original_save_loads_as_the_original_had_it_and_runs_on() {
	let Some(engine) = loads_as_the_original_had_it("an_original_save_loads", "new_campaign") else { return };
	script(&engine, "unpause", "::World.State.setAutoPause(false); ::World.State.setPause(false);");
	let start = player::eval_i64(&engine, "(::World.getTime().Time * 100).tointeger()").unwrap().unwrap();
	for _ in 0..1200 {
		frame(&engine);
	}
	let later = player::eval_i64(&engine, "(::World.getTime().Time * 100).tointeger()").unwrap().unwrap();
	assert!(later > start, "the loaded world's clock doesn't run ({start} → {later})");
}

/// The same campaign later (contracts made, a party more), and after the original loaded
/// and saved it again.
#[test]
fn other_original_saves_load_as_the_original_had_them() {
	loads_as_the_original_had_it("later_original_saves_load", "new_campaign_later");
	loads_as_the_original_had_it("reloaded_original_saves_load", "new_campaign_reloaded");
}
