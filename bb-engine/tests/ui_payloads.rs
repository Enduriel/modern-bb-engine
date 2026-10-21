//! UI payload parity, the scripts' side: the payloads the scripts send to the UI while a new
//! campaign's world is walked through bb-ui's world-golden steps (up to a bandit ambush's
//! first player turn), compared with the
//! original's (`parity/ui_payloads/world.nut` records them in both engines;
//! `parity/capture_ui_payloads.py` captures the original's into
//! `parity/goldens/ui_payloads/world.txt`).
//!
//! The campaign starts from the main menu as the probe harness's campaign mode starts
//! it, the recorder runs right after `world_state.init()`, and the steps run 3 s of
//! frames apart, as the original's driver waits 3 s between them. Known differences are
//! listed in `parity/divergences.txt` under `ui_payloads/world`. `PARITY_DUMP=<dir>`
//! writes our output there. Skips if the Steam install is missing.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use bb_engine::harness::{self, GameEngine};
use squirrel::SqTable;
use squirrel_bb::LogLevel;

#[path = "support/parity.rs"]
mod common;

use common::{Report, compare, entries, finish, golden_lines, load_divergences, normalize, parity_dir};

const PROBE: &str = "ui_payloads/world";

/// The map seed: bb-ui's world goldens' (the probe's `--seed`).
const SEED: &str = "bbuitest";

/// How the probe harness's campaign mode starts the campaign, as
/// bb-ui's `host::goldens` does too.
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
	Seed = "@SEED@",
	StartingScenario = "scenario.early_access"
});"#;

const FRAME_MS: f64 = 16.0;

fn frames(engine: &GameEngine, seconds: f64) {
	for _ in 0..(seconds * 1000.0 / FRAME_MS).round() as usize {
		harness::run_frame(engine, FRAME_MS).expect("a frame");
	}
}

/// Evaluate a boolean script expression and read the result back.
fn eval_bool(engine: &GameEngine, expr: &str) -> bool {
	engine
		.run_script(&format!("::__probeResult <- {expr};"), "ui_payloads")
		.unwrap_or_else(|e| panic!("{expr}: {e}"));
	SqTable::root(engine.vm())
		.get::<bool>("__probeResult")
		.unwrap_or_else(|| panic!("{expr} isn't a bool"))
}

/// Our engine's PROBE lines, from the recorder's section marker on.
fn run_ours() -> Vec<String> {
	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	let lines = Rc::new(RefCell::new(Vec::new()));
	let sink = lines.clone();
	engine.set_log_handler(move |level, message| {
		if level == LogLevel::Info && message.starts_with("PROBE") {
			sink.borrow_mut().push(normalize(message));
		}
	});
	harness::boot_menu(&engine).expect("boot the menu");
	engine
		.run_script(&START_CAMPAIGN.replace("@SEED@", SEED), "start")
		.expect("start the campaign");

	// The recorder runs right after world_state.init(), before the world's first update:
	// a frame inits the states added since the last one and then updates them.
	let mut started = false;
	for _ in 0..(120_000.0 / FRAME_MS) as usize {
		engine.init_pending_states().expect("init the new states");
		if engine.root_state().state("WorldState").is_some() {
			let recorder = std::fs::read_to_string(parity_dir().join("ui_payloads/world.nut")).unwrap();
			engine.run_script(&recorder, "ui_payloads/world.nut").expect("the recorder runs");
			started = true;
			break;
		}
		harness::run_frame(&engine, FRAME_MS).expect("a frame");
	}
	assert!(started, "the campaign's world never started");

	let mut waited = 0.0;
	while !eval_bool(&engine, "::World.State.m.WorldScreen.probePayloadReady()") {
		assert!(waited < 120.0, "no intro event after 120 s");
		frames(&engine, 0.5);
		waited += 0.5;
	}
	frames(&engine, 3.0);
	loop {
		engine
			.run_script("::__probeStep <- ::World.State.m.WorldScreen.probePayloadStep();", "ui_payloads")
			.expect("a step");
		let result = SqTable::root(engine.vm()).get::<String>("__probeStep").expect("a step result");
		match result.as_str() {
			"next" => frames(&engine, 3.0),
			"waitPlayerTurn" => {
				let mut waited = 0.0;
				while !eval_bool(&engine, "::World.State.m.WorldScreen.probePayloadPlayerTurn()") {
					assert!(waited < 180.0, "no player turn after 180 s");
					frames(&engine, 0.5);
					waited += 0.5;
				}
				frames(&engine, 3.0);
			}
			_ => break,
		}
	}
	let all = lines.borrow().clone();
	let start = all
		.iter()
		.position(|l| l == &format!("PROBE_SECTION {PROBE}"))
		.expect("the recorder's section marker");
	all[start + 1..].to_vec()
}

#[test]
fn world_ui_payloads_match_the_original() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		bb_engine::harness::skip(format!("no install at {}", harness::STEAM_DATA_DIR));
		return;
	}
	let ours = run_ours();
	if let Some(dump) = std::env::var_os("PARITY_DUMP") {
		let path = Path::new(&dump).join(format!("{PROBE}.txt"));
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, ours.join("\n") + "\n").unwrap();
	}
	let mut report = Report::default();
	let allowed = load_divergences();
	match golden_lines(&parity_dir().join("goldens").join(format!("{PROBE}.txt"))) {
		Some(golden) => compare(PROBE, &entries(&golden), &entries(&ours), &allowed, false, &mut report),
		None => report.uncaptured.push(PROBE.to_string()),
	}
	finish(report, &allowed);
}
