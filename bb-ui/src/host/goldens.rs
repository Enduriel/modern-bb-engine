//! The UI parity goldens: which screens they show, how our page gets to
//! each one, and what their comparison leaves out. Captured from the original with
//! `bb-ui/probes/ui_goldens.js` (the menus) and `world_golden.js` (the world screen),
//! which walk the same paths.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use squirrel::SqTable;

use super::UiSession;
use crate::json::Json;
use crate::parity::{Snapshot, Tolerance};

/// The screens, in capture order, each with the clicks that lead to it from the
/// previous one.
pub const SCREENS: [(&str, &[&str]); 3] = [
	("main_menu", &[]),
	("new_campaign", &["New Campaign"]),
	("options", &["Cancel", "Options"]),
];

/// Where the goldens are kept in the repo.
pub fn default_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens")
}

/// Read screen `name`'s golden from `dir`.
pub fn load(dir: &Path, name: &str) -> Result<Snapshot, String> {
	let path = dir.join(format!("{name}.json"));
	let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
	Snapshot::parse(&Json::parse(&text)?)
}

/// What screen `name`'s comparison leaves out: the Options resolution list shows the
/// machine's display modes (none on the probe's virtual display), and the battle's
/// combat log has lines a pixel shorter here (the original rounds a font's total line
/// height once, Chromium each part), which adds up down the
/// log. Its text is still compared.
pub fn tolerance(name: &str) -> Tolerance {
	let mut tolerance = Tolerance::default();
	match name {
		"options" => tolerance
			.ignore_paths
			.push("div.control.l-list-container".into()),
		"world_combat" => tolerance
			.ignore_boxes
			.push("div.l-event-logs-container".into()),
		_ => {}
	}
	tolerance
}

/// The world's goldens, in capture order (`probes/world_golden.js`): right after a new
/// campaign, the origin's intro event; after its "Huzzah!", the world screen; the
/// character screen; after closing it, the nearest town's screen; and after leaving
/// that, a bandit ambush once one of the player's men is up.
pub const WORLD_SCREENS: [&str; 5] = [
	"world_intro",
	"world",
	"world_character",
	"world_town",
	"world_combat",
];

/// How long the ambush may take to reach the player's turn.
const PLAYER_TURN_TIMEOUT: Duration = Duration::from_secs(180);

/// The map seed of the world's campaign (the probe's `--seed`).
pub const WORLD_SEED: &str = "bbuitest";

/// The steps between the world's snapshots, which the probe runs too.
const WORLD_STEPS: &str = include_str!("../../probes/world_golden.nut");

/// How the probe harness's campaign mode starts the campaign, with the
/// seed left as `{seed}`.
const START_CAMPAIGN: &str = r#"::RootState.get("MainMenuState").campaign_menu_module_onStartPressed({
	Name = "Probe Company",
	Banner = ::Const.PlayerBanners[0],
	Difficulty = 1,
	EconomicDifficulty = 1,
	BudgetDifficulty = 1,
	Ironman = false,
	ExplorationMode = false,
	GreaterEvil = ::Const.World.GreaterEvilType.Random,
	PermanentDestruction = false,
	Seed = {seed},
	StartingScenario = "scenario.early_access"
});"#;

/// Start the world's campaign from the main menu as the golden probe's harness does,
/// and walk it through [`WORLD_SCREENS`] as the probe does, snapshotting each 3 s
/// after its step.
pub fn capture_world(session: &UiSession) -> Result<Vec<(&'static str, Snapshot)>, String> {
	let seed = Json::Str(WORLD_SEED.to_string()).to_text();
	let engine = session.engine();
	engine.run_script(&START_CAMPAIGN.replace("{seed}", &seed), "world_golden")?;
	// The probe harness runs the steps' .nut right after world_state.init(), before the
	// world's first update (it pins the generator there): do the same. A frame inits the
	// states added since the last one and then updates them, so init them first.
	let deadline = Instant::now() + Duration::from_secs(120);
	loop {
		engine.init_pending_states()?;
		if engine.root_state().state("WorldState").is_some() {
			engine.run_script(WORLD_STEPS, "world_golden.nut")?;
			break;
		}
		if Instant::now() >= deadline {
			return Err("the campaign's world never started".into());
		}
		session.run_frames(0.016)?;
	}
	session.wait_for_text("Huzzah!", Duration::from_secs(120))?;
	let step = |name: &str| {
		engine.run_script(
			&format!("::World.State.m.WorldScreen.probeStep(\"{name}\");"),
			"world_golden",
		)
	};

	let mut snapshots = Vec::new();
	let mut settle_and_snapshot = |name: &'static str| -> Result<(), String> {
		session.run_frames(3.0)?;
		snapshots.push((name, session.snapshot()?));
		Ok(())
	};
	settle_and_snapshot(WORLD_SCREENS[0])?;
	session.click_when_ready("Huzzah!", Duration::from_secs(30))?;
	// The probe clicks with jQuery: there's no mouse over its page to hover anything.
	session.move_away()?;
	settle_and_snapshot(WORLD_SCREENS[1])?;
	step("character")?;
	settle_and_snapshot(WORLD_SCREENS[2])?;
	step("closeCharacter")?;
	session.run_frames(3.0)?;
	step("town")?;
	settle_and_snapshot(WORLD_SCREENS[3])?;
	step("leaveTown")?;
	session.run_frames(3.0)?;
	step("combat")?;
	wait_for_player_turn(session)?;
	settle_and_snapshot(WORLD_SCREENS[4])?;
	Ok(snapshots)
}

/// Run the game until the battle waits for the player (the `playerTurn` step), as the
/// probe polls for it.
fn wait_for_player_turn(session: &UiSession) -> Result<(), String> {
	let deadline = Instant::now() + PLAYER_TURN_TIMEOUT;
	loop {
		let engine = session.engine();
		engine.run_script(
			"::__bbPlayerTurn <- ::World.State.m.WorldScreen.probeStep(\"playerTurn\");",
			"world_golden",
		)?;
		if SqTable::root(engine.vm()).get::<bool>("__bbPlayerTurn") == Some(true) {
			return Ok(());
		}
		if Instant::now() >= deadline {
			return Err(format!(
				"no player turn in the ambush after {PLAYER_TURN_TIMEOUT:?}; states {:?}",
				engine.root_state().state_names()
			));
		}
		session.run_frames(0.5)?;
	}
}

/// Walk our page through [`SCREENS`] the way the golden probe does, snapshotting each
/// once it has settled.
pub fn capture(session: &UiSession) -> Result<Vec<(&'static str, Snapshot)>, String> {
	let step = Duration::from_secs(30);
	let mut snapshots = Vec::new();
	for (name, clicks) in SCREENS {
		for click in clicks {
			session.click_when_ready(click, step)?;
		}
		// As long as the probe waits: menus slide in, and lists get their scrollbars.
		session.run_frames(3.0)?;
		snapshots.push((name, session.snapshot()?));
	}
	Ok(snapshots)
}
