//! A modded battle's turn bar on the real UI: with Modern Hooks and MSU mounted
//! (the reference mod corpus's, `bb_engine::mod_corpus`), a new campaign's company fights
//! bandits and takes the first turn, the battle the original locks up in. There the
//! turn bar logs "ERROR: Failed to query entity data for entity (N). Reason: Invalid
//! result." and stays locked, with no entity active: the original sends stray empty
//! `_Result` replies with older request ids, and with MSU's calls one lands on the
//! first slot's `onEntityEntersFirstSlot` call while the turn it starts queries the
//! entity's skills from the page (probed 2026-10-22, the same campaign seed and
//! battle). Ours answers each request once, so the turn bar
//! fills, the company's brother is active, and its turns go on to the next entities.
//! No reply reaches the page twice or without a value (an intentional divergence,
//! docs/differences.md). Skips without a Steam install or without the corpus; fails if
//! it is not the pinned one.
//!
//! Run: `cargo test -p bb-ui --features cef --test mods_battle_ui`
//!
//! A plain `main` (`harness = false`): Chromium re-runs this binary for its child
//! processes, and CEF can only start once per process.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_engine::harness::STEAM_DATA_DIR;
use bb_engine::mod_corpus::{Corpus, CorpusError};
use bb_engine::tactical_player as player;
use bb_ui::host::{Echo, HostConfig, UiSession};
use bb_ui::json::Json;

#[path = "support/gpu_page.rs"]
mod gpu_page;

const NAME: &str = "a_modded_battles_first_turn_fills_the_turn_bar";
/// The mods' zips (name prefixes): the original's failing run had these two.
const MODS: [&str; 2] = ["mod_modern_hooks", "mod_msu"];
/// The original's failing run's campaign seed (the probe harness's `--seed e2e`).
const SEED: &str = "e2e";
/// The turn bar's error in vanilla's `turnsequencebar_module.js`.
const QUERY_FAILED: &str = "Failed to query entity data";

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition { Ok(()) } else { Err(what.to_string()) }
}

/// A Squirrel condition, evaluated in the scripts.
fn holds(session: &UiSession, expr: &str) -> Result<bool, String> {
	Ok(player::eval_i64(session.engine(), &format!("({expr}) ? 1 : 0"))? == Some(1))
}

/// Run the game in real time until the scripts' `cond` holds.
fn until(session: &UiSession, cond: &str, timeout: Duration) -> Result<(), String> {
	let deadline = Instant::now() + timeout;
	loop {
		if holds(session, cond)? {
			return Ok(());
		}
		ensure(Instant::now() < deadline, format!("still not true after {timeout:?}: {cond}"))?;
		session.run_frames(0.1)?;
	}
}

/// The corpus's mod storages (MSU's settings), copied: never written back.
fn copy_mod_storages(corpus: &Corpus) -> Result<PathBuf, String> {
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mods_battle_ui_storages");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
	corpus.copy_storages(&dir).map_err(|e| format!("the mods' storages: {e}"))?;
	Ok(dir)
}

/// Every reply that reaches the page's `engine._Result` is kept (its request id and how
/// many arguments it had), and every `callSQ` the page makes (its id, the script
/// method, and the id of the `callSQ` it was made inside, or null).
const KEEP_REPLIES: &str = r#"(function () {
	window.__bbReplies = [];
	window.__bbCalls = [];
	var result = engine._Result, call = engine.call, inside = [];
	engine._Result = function () {
		__bbReplies.push([arguments[0], arguments.length]);
		return result.apply(this, arguments);
	};
	engine.call = function (name, handle, method) {
		if (name !== 'callSQ') {
			return call.apply(this, arguments);
		}
		var id = engine._RequestId + 1;
		__bbCalls.push([id, method, inside.length ? inside[inside.length - 1] : null]);
		inside.push(id);
		try {
			return call.apply(this, arguments);
		} finally {
			inside.pop();
		}
	};
	return 0;
})()"#;

/// The original's failing probe's battle: the intro event answered, the company's
/// Initiative raised so a brother takes the first turn, bandits.
const BATTLE: &str = r#"
	local screen = ::World.State.getEventScreen();
	if (screen != null && screen.isVisible()) screen.onButtonPressed(0);
	foreach (bro in ::World.getPlayerRoster().getAll()) bro.getBaseProperties().Initiative = 1000;
	local st = ::World.State;
	local p = st.getLocalCombatProperties(st.getPlayer().getPos());
	p.CombatID = "Event";
	p.IsAutoAssigningBases = false;
	p.Entities = [];
	::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditDefenders, 100, ::Const.Faction.Enemy);
	st.startScriptedCombat(p, false, false, true);
"#;

/// The turn bar's active entity: its id, or -1 while there is none (the bar locked).
const ACTIVE: &str = "(function() { local a = ::Tactical.TurnSequenceBar.getActiveEntity(); return a == null ? -1 : a.getID(); })()";

fn in_battle(session: &UiSession) -> Result<(), String> {
	// The campaign, started as the probe harness starts it; MSU's new-campaign settings
	// page, if it shows (only mods with such settings bring it up), goes on with it.
	session.start_campaign(SEED)?;
	let deadline = Instant::now() + Duration::from_secs(300);
	let mut pressed_save = false;
	loop {
		ensure(Instant::now() < deadline, "the world never came up")?;
		session.run_frames(0.1)?;
		if !pressed_save && holds(session, "::MSU.SettingsScreen.isVisible()")? {
			session.engine().run_script("::MSU.SettingsScreen.onSaveButtonPressed({});", "msu settings")?;
			pressed_save = true;
		}
		let up = "\"State\" in ::World && ::World.State != null && ::World.State.getPlayer() != null \
			&& !::World.State.isInLoadingScreen() && !::LoadingScreen.isVisible()";
		if holds(session, up)? {
			break;
		}
	}
	// The intro event, as the probe waited for it.
	until(session, "(function() { local s = ::World.State.getEventScreen(); return s != null && s.isVisible() && !s.isAnimating(); })()", Duration::from_secs(60))?;
	session.eval(KEEP_REPLIES)?;
	session.engine().run_script(BATTLE, "battle")?;
	until(
		session,
		"(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible()",
		Duration::from_secs(120),
	)
}

fn first_turn(session: &UiSession) -> Result<(), String> {
	in_battle(session)?;
	// The first turn: the bar unlocks with a brother in its first slot.
	until(session, &format!("{ACTIVE} >= 0"), Duration::from_secs(60)).map_err(|e| format!("the turn bar stays locked: {e}"))?;
	ensure(
		holds(session, "::Tactical.TurnSequenceBar.getActiveEntity().isPlayerControlled()")?,
		"the first turn isn't a brother's (the original's failing case)",
	)?;
	// The page has the brother's turn: its first slot holds the entity and the bar's
	// buttons show for the player.
	let first = player::eval_i64(session.engine(), ACTIVE)?.unwrap_or(-1);
	// The brothers' turns end as the probe's did; the bar goes on to the next entities.
	let mut active = first;
	for turn in 0..3 {
		session.engine().run_script("::Tactical.TurnSequenceBar.onNextTurnButtonPressed();", "end turn")?;
		let was = active;
		until(session, &format!("{ACTIVE} >= 0 && {ACTIVE} != {was}"), Duration::from_secs(60))
			.map_err(|e| format!("turn {}: the turn bar didn't go on from entity {was}: {e}", turn + 1))?;
		active = player::eval_i64(session.engine(), ACTIVE)?.unwrap_or(-1);
		if !holds(session, "::Tactical.TurnSequenceBar.getActiveEntity().isPlayerControlled()")? {
			break;
		}
	}
	session.run_frames(1.0)?;
	let failed: Vec<String> = session
		.log()
		.into_iter()
		.filter(|line| line.text.contains(QUERY_FAILED) || line.level == "bridge")
		.map(|line| format!("[{}] {}", line.level, line.text))
		.collect();
	ensure(failed.is_empty(), format!("the page logged:\n{}", failed.join("\n")))?;
	// Each request answered once, with a value.
	let replies = session.eval("__bbReplies")?;
	let replies = replies.as_array().unwrap_or_default();
	ensure(!replies.is_empty(), "no replies reached the page during the battle")?;
	let mut ids = Vec::new();
	for reply in replies {
		let pair = reply.as_array().unwrap_or_default();
		let id = pair.first().and_then(Json::as_i64);
		ensure(!ids.contains(&id), format!("request {id:?} was answered twice"))?;
		ensure(pair.get(1).and_then(Json::as_i64) == Some(2), format!("a reply without a value: {}", reply.to_text()))?;
		ids.push(id);
	}
	// The original's failing case: the first slot's call, answered, with the turn's
	// own calls back into the scripts made inside it.
	let calls = session.eval("__bbCalls")?;
	let calls: Vec<(Option<i64>, String, Option<i64>)> = calls
		.as_array()
		.unwrap_or_default()
		.iter()
		.map(|c| {
			let c = c.as_array().unwrap_or_default();
			let id = c.first().and_then(Json::as_i64);
			let method = c.get(1).and_then(Json::as_str).unwrap_or_default().to_string();
			(id, method, c.get(2).and_then(Json::as_i64))
		})
		.collect();
	let enters = calls.iter().find(|c| c.1 == "onEntityEntersFirstSlot").map(|c| c.0);
	ensure(enters.is_some(), "the page never called onEntityEntersFirstSlot")?;
	ensure(ids.contains(&enters.flatten()), "the first slot's call was never answered")?;
	let nested: Vec<&str> = calls.iter().filter(|c| c.2 == enters.flatten()).map(|c| c.1.as_str()).collect();
	ensure(
		!nested.is_empty(),
		"no callSQ was made inside the first slot's call (not the original's failing case)",
	)?;
	println!(
		"  first turn: entity {first}; {} replies, each once; inside the first slot's call: {nested:?}",
		replies.len()
	);
	Ok(())
}

fn check(data_dir: &Path, corpus: &Corpus, mods: Vec<PathBuf>) -> Result<(), String> {
	let config = HostConfig { extra_mods: mods, sink: Some(gpu_page::sink()?), ..HostConfig::default() };
	let session = UiSession::start(data_dir, config, Echo::Off)?;
	session.engine().saves().set_directory(copy_mod_storages(corpus)?);
	session.boot_menu()?;
	first_turn(&session).map_err(|error| {
		let shot = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mods_battle_ui-failure.png");
		let saved = match session.screenshot(&shot) {
			Ok(Some(_)) => format!("\nlast frame: {}", shot.display()),
			_ => String::new(),
		};
		let errors: Vec<String> = session
			.log()
			.into_iter()
			.filter(|line| line.level == "error" || line.level == "bridge")
			.map(|line| format!("[{}] {}", line.level, line.text))
			.collect();
		format!("{error}\npage errors:\n{}{saved}", errors.join("\n"))
	})
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// A runner's `--list` (cargo-nextest's), answered before a display starts.
	if let Some(code) = bb_engine::harness::list_case(NAME) {
		return code;
	}
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	let data_dir = Path::new(STEAM_DATA_DIR);
	if !data_dir.exists() {
		return bb_engine::harness::skip_case(NAME, format!("{STEAM_DATA_DIR} not present"));
	}
	let corpus = match bb_engine::mod_corpus::reference() {
		Ok(corpus) => corpus,
		Err(CorpusError::Absent(why)) => return bb_engine::harness::skip_case(NAME, why),
		Err(e) => {
			println!("test {NAME} ... FAILED\n{e}");
			return ExitCode::FAILURE;
		}
	};
	let mut mods = Vec::new();
	for prefix in MODS {
		match corpus.find(prefix) {
			Some(zip) => mods.push(zip.clone()),
			None => {
				println!("test {NAME} ... FAILED\nno {prefix}* mod in the reference mod corpus");
				return ExitCode::FAILURE;
			}
		}
	}
	match check(data_dir, &corpus, mods) {
		Ok(()) => {
			println!("test {NAME} ... ok");
			ExitCode::SUCCESS
		}
		Err(error) => {
			println!("test {NAME} ... FAILED\n{error}");
			ExitCode::FAILURE
		}
	}
}
