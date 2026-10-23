//! Shared headless boot steps for the real game, used by the examples and the
//! integration tests: open an install's archives, load config and DLC, boot the menu,
//! and drive the genuine new-campaign sequence.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::{fmt, io};

use squirrel_bb::{Engine, GameSettings, NativeConfig, installed_dlc};

use crate::assets::{load_fog_stamp, load_sprites};
use crate::install;
use crate::mounts::Mounts;
use crate::probe_aside;
use crate::{DatScriptSource, ScriptCache};

/// The game's data directory: `$BB_DATA_DIR`, else that of this machine's Steam install
/// ([`install::steam_data_dir`]), else [`NO_INSTALL`] (a path that doesn't exist, so a
/// test checking for it says why it can't run). Looked up once.
pub static STEAM_DATA_DIR: &EnvDir = &STEAM_DATA;
static STEAM_DATA: EnvDir = EnvDir::new(install::DATA_DIR_VAR, || {
	install::steam_data_dir().map_or_else(|| NO_INSTALL.to_string(), |dir| dir.to_string_lossy().into_owned())
});

/// [`STEAM_DATA_DIR`] on a machine without the game: no Steam install, and no
/// `BB_DATA_DIR`.
pub const NO_INSTALL: &str = "<no Battle Brothers install: Steam has none, and BB_DATA_DIR is unset>";

/// Saves the original wrote, for the tests of loading and writing its format: its
/// campaigns as capture probes saved them, with its own dumps of their state (see the
/// README there): `$BB_ORIGINAL_SAVES` (unset: a path that doesn't exist). Not in the
/// repository (each save is ~6.7 MB); tests skip without them.
pub static ORIGINAL_SAVES_DIR: &EnvDir = &ORIGINAL_SAVES;
static ORIGINAL_SAVES: EnvDir = EnvDir::new("BB_ORIGINAL_SAVES", || "<BB_ORIGINAL_SAVES is unset>".to_string());

/// Says a test may skip when this machine lacks what it needs (the Steam install, a
/// mod, a GPU, gamescope): `BB_ALLOW_SKIPS=1`, for machines without them. Unset, a test
/// that would skip fails instead, so a green run means every test ran (libtest hides a
/// passing test's output: a quiet skip looks like a pass).
pub const ALLOW_SKIPS: &str = "BB_ALLOW_SKIPS";

/// Whether skipping is allowed ([`ALLOW_SKIPS`] is `1`).
pub fn skips_allowed() -> bool {
	std::env::var_os(ALLOW_SKIPS).is_some_and(|v| v == "1")
}

/// The error a test that would skip for `why` fails with, unless skips are allowed.
fn refused(why: &impl fmt::Display) -> Option<String> {
	(!skips_allowed()).then(|| format!("this test can't run here: {why} (set {ALLOW_SKIPS}=1 to skip it on a machine without it)"))
}

/// A libtest test can't run on this machine because `why` (what it lacks): with skips
/// allowed ([`ALLOW_SKIPS`]) this prints `skipping: <why>` and the test then returns;
/// otherwise the test fails here.
#[track_caller]
pub fn skip(why: impl fmt::Display) {
	if let Some(e) = refused(&why) {
		panic!("{e}");
	}
	eprintln!("skipping: {why}");
}

/// [`skip`] for a test of a `harness = false` binary, named `name`: prints its
/// libtest-style line (`test <name> ... skipping: <why>`, or `... FAILED` and why) and
/// returns the binary's exit status.
pub fn skip_case(name: &str, why: impl fmt::Display) -> std::process::ExitCode {
	match refused(&why) {
		None => {
			println!("test {name} ... skipping: {why}");
			std::process::ExitCode::SUCCESS
		}
		Some(e) => {
			println!("test {name} ... FAILED\n{e}");
			std::process::ExitCode::FAILURE
		}
	}
}

/// For the `main` of a `harness = false` binary, first thing (before it starts a display
/// or a browser): answers a runner's `--list` as libtest does, so cargo-nextest can run
/// the binary (`scripts/test_all.py`). The binary is one test, named `name`, never
/// ignored: `--list` prints `<name>: test`, `--list --ignored` nothing. Returns `None`
/// without `--list`; then the binary runs as it would (it takes no filter: nextest
/// passes `<name> --exact`, which names the one test there is).
pub fn list_case(name: &str) -> Option<std::process::ExitCode> {
	let args: Vec<String> = std::env::args().skip(1).collect();
	if !args.iter().any(|a| a == "--list") {
		return None;
	}
	if !args.iter().any(|a| a == "--ignored") {
		println!("{name}: test");
	}
	Some(std::process::ExitCode::SUCCESS)
}

/// A directory the environment can override: the variable's value if it's set, else a
/// default (made on first use); read on first use. Usable where the `&str` constants it replaced were
/// (`Path::new`, `PathBuf::from`, formatting).
pub struct EnvDir {
	var: &'static str,
	default: fn() -> String,
	path: OnceLock<String>,
}

impl EnvDir {
	const fn new(var: &'static str, default: fn() -> String) -> Self {
		EnvDir { var, default, path: OnceLock::new() }
	}

	pub fn as_str(&self) -> &str {
		self.path.get_or_init(|| {
			std::env::var_os(self.var)
				.and_then(|v| v.into_string().ok())
				.unwrap_or_else(self.default)
		})
	}
}

impl AsRef<OsStr> for EnvDir {
	fn as_ref(&self) -> &OsStr {
		self.as_str().as_ref()
	}
}

impl AsRef<Path> for EnvDir {
	fn as_ref(&self) -> &Path {
		self.as_str().as_ref()
	}
}

impl fmt::Display for EnvDir {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

/// An engine over a real install.
pub type GameEngine = Engine<DatScriptSource>;

/// The `data_*.dat` archives in `dir`, in load order.
pub fn data_archives(dir: &Path) -> io::Result<Vec<PathBuf>> {
	let mut dats: Vec<PathBuf> = std::fs::read_dir(dir)?
		.flatten()
		.map(|e| e.path())
		.filter(|p| {
			p.file_name().is_some_and(|n| {
				let n = n.to_string_lossy();
				n.starts_with("data_") && n.ends_with(".dat")
			})
		})
		.collect();
	dats.sort();
	Ok(dats)
}

/// The installed mods in `dir`: every `.zip`, in the order the original mounts them
/// after the `data_*.dat` archives (by name; a later archive wins).
///
/// While a probe run is up, `data/` is not what was installed: the run holds the
/// installed mods aside and may copy others in. The list is the installed mods all
/// the same ([`probe_aside::user_mods`]), so a probe run changes nothing for those
/// mounting them.
pub fn mod_archives(dir: &Path) -> io::Result<Vec<PathBuf>> {
	let mut zips = probe_aside::user_mods(dir)?;
	zips.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
	Ok(zips)
}

/// `mods` (in load order, as [`mod_archives`] lists them) with the zips at `extra` added,
/// wherever they are on disk: all of them in load order by file name, as the original
/// would mount them had they been copied into `data/`. An extra zip named like a listed
/// mod takes its place, as a copy into `data/` would replace the file (e.g. a dev build
/// of an installed mod); two extras of the same name are an error.
pub fn with_extra_mods(mut mods: Vec<PathBuf>, extra: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
	for (i, zip) in extra.iter().enumerate() {
		let name = zip.file_name().ok_or_else(|| format!("not a file: {}", zip.display()))?;
		if extra[..i].iter().any(|z| z.file_name() == Some(name)) {
			return Err(format!("two extra mods named {}", name.to_string_lossy()));
		}
		mods.retain(|m| m.file_name() != Some(name));
	}
	mods.extend(extra);
	mods.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
	Ok(mods)
}

pub use crate::probe_aside::{HELD_MODS, locate_mod};

/// An engine over the install at `dir` with its mods mounted ([`mod_archives`]), as the
/// original loads them.
pub fn open_install_with_mods(dir: &Path) -> io::Result<GameEngine> {
	let mut archives = data_archives(dir)?;
	archives.extend(mod_archives(dir)?);
	open_mounted(dir, archives, |_| {})
}

/// The gameplay settings of the install the original's ground truth was probed on (its
/// `config.cfg`), with a fresh install's other settings: the harness's engines start
/// with them, so they behave as the goldens were recorded. A fresh install has fewer
/// gameplay options on ([`GameSettings::FRESH_INSTALL`]); the windowed game uses its
/// settings file's.
pub const PROBED_INSTALL_SETTINGS: GameSettings = GameSettings {
	// AlwaysFocusCamera, AdjustCameraLevel, ShowOverlayStats, ShowOrientationOverlays,
	// FasterPlayerMovement, FasterAIMovement, AutoLoot, AlwaysHideTrees,
	// DontAutoEndTurns, AutoPauseAfterCity, RestoreEquipment.
	gameplay: [false, true, true, true, true, true, false, true, true, true, true],
	..GameSettings::FRESH_INSTALL
};

/// An engine over the install at `dir`, configured as the original sees a Steam install:
/// DLC ownership is whichever DLC archives Steam installed (each ships its own
/// `scripts/dlc/` init script). Transcoded scripts are cached on disk as
/// [`ScriptCache::from_env`] configures (`BB_SCRIPT_CACHE=off` disables it). The sprite
/// sheets are loaded, as the original does before any script runs; a sheet that fails
/// to load is reported on stderr, and so is a missing fog-of-war stamp. The engine's own random stream is unpredictably
/// seeded like the original's, unless `BB_ENGINE_SEED` gives a seed (for reproducible renders).
pub fn open_install(dir: &Path) -> io::Result<GameEngine> {
	open_mounted(dir, data_archives(dir)?, |_| {})
}

/// [`open_install`], letting the caller adjust the engine's [`NativeConfig`] first. The
/// windowed game sets `exact_clock` to [`ExactClock::Live`](squirrel_bb::ExactClock), as
/// the original's clock is; the harness and tests keep frame time.
pub fn open_install_with(dir: &Path, configure: impl FnOnce(&mut NativeConfig)) -> io::Result<GameEngine> {
	open_mounted(dir, data_archives(dir)?, configure)
}

/// An engine over `archives` (in mount order) with the data folder `dir` and the install
/// folder above it mounted beside them ([`Mounts`]), configured as [`open_install`] says,
/// then by `configure`.
pub fn open_mounted(
	dir: &Path,
	archives: Vec<PathBuf>,
	configure: impl FnOnce(&mut NativeConfig),
) -> io::Result<GameEngine> {
	let mut mounts = vec![dir.to_path_buf()];
	mounts.extend(dir.parent().map(Path::to_path_buf));
	let archives: Vec<PathBuf> = archives.iter().map(|a| locate_mod(a)).collect();
	let source = DatScriptSource::open(archives)?
		.with_cache(ScriptCache::from_env())
		.with_mounts(Mounts::new(mounts));
	let sprites = source.with_archives(|archives| load_sprites(archives));
	for failure in &sprites.failures {
		eprintln!("[Error] brush {failure}");
	}
	let mut config = NativeConfig {
		is_steam_build: true,
		owned_dlc: installed_dlc(&source),
		engine_seed: std::env::var("BB_ENGINE_SEED").ok().and_then(|s| s.parse().ok()),
		..NativeConfig::default()
	};
	configure(&mut config);
	let fog_stamp = source.with_archives(|archives| load_fog_stamp(archives, &sprites.registry));
	let engine = Engine::with_config(source, &config);
	// The settings the parity goldens were captured with (the probed install's).
	if let Err(e) = engine.apply_game_settings(&PROBED_INSTALL_SETTINGS) {
		eprintln!("[Error] settings: {e}");
	}
	engine.sprites().set(sprites.registry);
	match fog_stamp {
		Ok(stamp) => engine.world_grid().set_fog_stamp(stamp),
		Err(e) => eprintln!("[Error] fog of war: {e}"),
	}
	Ok(engine)
}

/// Run the game's own `scripts/preload`, as the original does before the menu: config
/// tables, DLC init scripts, then every script under `scripts/` (some define globals at
/// file level through `getroottable()`, e.g. the contract negotiation templates).
pub fn load_config(engine: &GameEngine) -> Result<(), String> {
	engine.run_script("::include(\"scripts/preload\");", "preload")
}

/// Boot to the main menu: config, then `scripts/root_state`. Then park the cursor, as
/// the first mouse event would, so frame code that reads it has a position (over the
/// UI, see `UI.setCursorPosition`). With Modern Hooks loaded, root_state's own onInit,
/// which makes `::Cursor`, waits for MH's UI handshake, which the UI client answers on
/// a pump.
pub fn boot_menu(engine: &GameEngine) -> Result<(), String> {
	boot_menu_with(engine, 4, || engine.pump_ui().map(|_| ()))
}

/// [`boot_menu`], with `wait` run (up to `tries` times) until root_state's own onInit has
/// made `::Cursor`. A host with a real UI passes one that runs the page, where Modern
/// Hooks' handshake then happens (it loads every mod's JS first).
pub fn boot_menu_with(
	engine: &GameEngine,
	tries: u32,
	wait: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
	load_config(engine)?;
	squirrel_bb::startup::mark("preload run");
	boot_root_state_with(engine, tries, wait)
}

/// The rest of [`boot_menu_with`] after the config (`load_config`): `scripts/root_state`,
/// the wait for the UI, the cursor.
pub fn boot_root_state_with(
	engine: &GameEngine,
	tries: u32,
	mut wait: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
	engine.boot("scripts/root_state")?;
	squirrel_bb::startup::mark("root state booted");
	for _ in 0..tries {
		if squirrel::SqTable::root(engine.vm()).get::<squirrel::SqTable>("Cursor").is_some() {
			break;
		}
		wait()?;
	}
	engine.run_script("::Cursor.setPosition(0, 0);", "cursor")?;
	// Let the UI answer the menu's screens (connected, shown) before anything else.
	engine.pump_ui().map(|_| ())
}

/// Add the world state and give it new-campaign settings for `seed`, ready for
/// [`start_new_campaign`]. The world UI is the real script UI, talking to the headless
/// UI bridge; the automatic `init()` is disabled so the caller drives the sequence.
/// The state is left in `::__bootWorldState`.
pub fn prepare_new_campaign(engine: &GameEngine, seed: &str) -> Result<(), String> {
	let script = r#"
		// Its onInit (run once it has its settings, see below) stands up the 11 managers,
		// the UI and the disabled init (patched on the state itself: the class isn't a
		// global, its file runs in its own scope).
		this.RootState.add("WorldState", "scripts/states/world_state");
		local ws = this.RootState.get("WorldState");
		ws.init <- function() {};
		ws.setNewCampaignSettings({
			Name = "Boot Test Company",
			Banner = "banner_04",
			Difficulty = 1,
			EconomicDifficulty = 1,
			BudgetDifficulty = 1,
			Ironman = false,
			PermanentDestruction = false,
			ExplorationMode = false,
			GreaterEvil = this.Const.World.GreaterEvilType.Random,
			Seed = "@SEED@",
			StartingScenario = this.new("scripts/scenarios/world/early_access_scenario")
		});
		::__bootWorldState <- ws;
		"#
	.replace("@SEED@", seed);
	engine.run_script(&script, "world_boot")?;
	// The new state initializes after it has been handed its settings, as in the
	// original (its next update).
	engine.init_pending_states()?;
	// Shown, as the end of its disabled `init()` would show it (without the `onShow`,
	// whose world screen the callers bring up themselves, if at all): an added state
	// runs its frames and gets input only once shown.
	if !engine.root_state().mark_shown("WorldState") {
		return Err("no WorldState to show".into());
	}
	Ok(())
}

/// Make the next campaigns' world map `cols` × `rows` tiles instead of vanilla's 140 × 170
/// (the variable world size): set `Const.World.Settings.SizeX/SizeY`, as a mod or the
/// console would. The world generator takes them when the next campaign starts (see
/// squirrel-bb's `world_size`), so this works at the main menu, between campaigns.
///
/// Vanilla's worldgen rejects every layout of a square map of 130 or less (its land and
/// desert minimums are absolute), so such a campaign never starts.
pub fn set_world_size(engine: &GameEngine, cols: i64, rows: i64) -> Result<(), String> {
	if cols < 1 || rows < 1 {
		return Err(format!("a {cols} x {rows} world map"));
	}
	engine.run_script(
		&format!("::Const.World.Settings.SizeX = {cols}; ::Const.World.Settings.SizeY = {rows};"),
		"world size",
	)
}

/// Run the genuine `startNewCampaign`: worldgen, assets, factions, the starting
/// scenario, and the faction simulation.
pub fn start_new_campaign(engine: &GameEngine) -> Result<(), String> {
	engine.run_script("::__bootWorldState.startNewCampaign();", "start")
}

/// Run a new campaign's world past its origin's intro event, if it hasn't shown yet:
/// every origin shows one 1 s of real time in (`Time.scheduleEvent(Real, 1000, …)`),
/// and the world pause it brings, were it to come during a battle, stops the battle's
/// clock (the world's speed is `Time`'s virtual speed), which a player can't be in that
/// soon. Runs frames until no script's real-time event is pending (the ambience's
/// timers are `Time` events too, always pending in the world) and the event screen is
/// closed, pressing its first button. Does nothing past the intro, so a test that
/// waited already keeps its timing. [`engage_nearest_hostile`] runs it; a test starting
/// a battle itself should too (the scripted player refuses a battle so stopped).
pub fn run_past_intro(engine: &GameEngine) -> Result<(), String> {
	const SHOWN: &str = "::World.State.getEventScreen().isVisible() ? 1 : 0";
	const ANSWER: &str = "local s = ::World.State.getEventScreen(); if (s.isVisible() && !s.isAnimating()) s.onButtonPressed(0);";
	for _ in 0..600 {
		if !engine.time().script_event_scheduled(0) && crate::tactical_player::eval_i64(engine, SHOWN)? != Some(1) {
			return Ok(());
		}
		engine.run_script(ANSWER, "answer the intro")?;
		run_frame(engine, 1000.0 / 60.0)?;
	}
	Err("the origin's intro event was still to come or answer after 10 s".into())
}

/// Let the world run, as a player does after starting a campaign: unpause, and from
/// then on let [`run_frame`]'s scripted player answer events.
pub fn unpause_world(engine: &GameEngine) -> Result<(), String> {
	install_autoplay(engine)?;
	engine.run_script(
		"::__bootWorldState.setAutoPause(false); ::__bootWorldState.setPause(false);",
		"unpause",
	)
}

/// Install the scripted player ([`AUTOPLAY`]) without unpausing the world; [`run_frame`]
/// runs it.
pub fn install_autoplay(engine: &GameEngine) -> Result<(), String> {
	engine.run_script(AUTOPLAY, "autoplay")
}

/// The scripted player: picks the first option of whatever event or contract screen
/// is open, as clicking its first button would. With `::__autoplayEngage` set, it also
/// engages when the combat dialog offers a battle. In a battle it ends each of its
/// units' turns at once (the turn bar's own guards hold the press while the bar is
/// busy), leaving the fighting to the AI, and leaves the result screen when it's over
/// (recording `::__lastCombatResult`). With `::__autoplayFight` set, the AI fights for
/// the player's units too, handed over by clearing `IsControlledByPlayer`. That isn't a
/// charm: a handed-over unit that dies stays on the player's roster with its items'
/// container cleared, and the world's `onCombatFinished` then fails on it, in the
/// original's scripts as in ours (a real charm's `onDeath` gives the unit back to the
/// player first; `tests/battle_deaths.rs`). The draw-exact traces keep that, as their
/// probe runs did. With `::__autoplayDieAsPlayers` set too, a handed-over unit is the
/// player's again as it dies, so its death is a brother's (off the roster, items
/// released), as when a player plays the battle: set it for any battle that goes back
/// to the world. Once the battle is over, the handed-over brothers are the player's
/// again (a player's agent), so a next battle hands them over anew. With `::__autoplayPlayer` set to a function, that function
/// plays the player's turns instead (it is called every frame of the battle).
const AUTOPLAY: &str = r#"
::__autoplayEngage <- false;
::__autoplayEngaged <- false;
::__autoplayFight <- false;
::__autoplayDieAsPlayers <- false;
::__autoplayPlayer <- null;
::__autoplay <- function()
{
	if (("State" in ::Tactical) && ::Tactical.State != null)
	{
		local st = ::Tactical.State;
		if (::__autoplayFight && !st.isBattleEnded())
		{
			// Hand the player's units to the AI (as a charm does), so both sides fight.
			foreach (bro in ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player))
			{
				if (!bro.m.IsControlledByPlayer) continue;
				local agent = ::new("scripts/ai/tactical/agents/military_melee_agent");
				agent.setActor(bro);
				bro.setAIAgent(agent);
				bro.m.IsControlledByPlayer = false;
				bro.m.__autoplayHanded <- true;
				if (::__autoplayDieAsPlayers)
				{
					local kill = bro.kill;
					bro.kill <- function( ... )
					{
						this.m.IsControlledByPlayer = true;
						vargv.insert(0, this);
						return kill.acall(vargv);
					};
				}
				// Its turn may already be under way: tell the new agent, as the turn
				// start does.
				if (bro.isTurnStarted()) agent.onTurnStarted();
			}
		}
		if (!st.isBattleEnded())
		{
			// Stopped by a world pause (speed 0, the battle not paused): the origin's intro
			// event, when the battle began within its 1 s (see `run_past_intro`).
			if (::Time.getVirtualSpeed() == 0 && !st.isPaused())
				throw "the battle's clock was stopped by a world pause (the origin's intro event, if the battle began within 1 s of the campaign: run harness::run_past_intro first)";
			if (::__autoplayPlayer != null)
			{
				::__autoplayPlayer();
			}
			else
			{
				local bar = ::Tactical.TurnSequenceBar;
				local active = bar.getActiveEntity();
				if (active != null && active.isPlayerControlled()) bar.onNextTurnButtonPressed();
			}
		}
		else
		{
			// The battle is over: the handed-over brothers are the player's again, with
			// a player's agent, as the next battle needs them.
			foreach (bro in ::World.getPlayerRoster().getAll())
			{
				if (!("__autoplayHanded" in bro.m)) continue;
				delete bro.m.__autoplayHanded;
				bro.m.IsControlledByPlayer = true;
				local agent = ::new("scripts/ai/tactical/player_agent");
				agent.setActor(bro);
				bro.setAIAgent(agent);
				if (bro.rawin("kill")) delete bro.kill;
			}
			// Leave once, as a player clicks once (the screen stays up a while after).
			local results = st.m.TacticalCombatResultScreen;
			if (!("__autoplayLeft" in st.m) && results != null && results.isVisible() && !results.isAnimating())
			{
				st.m.__autoplayLeft <- true;
				::__lastCombatResult <- ::Tactical.Entities.getCombatResult();
				results.onLeaveButtonPressed();
			}
		}
		return;
	}
	if (!("State" in ::World) || ::World.State == null) return;
	local state = ::World.State;
	local screen = state.getEventScreen();
	if (screen != null && screen.isVisible() && !screen.isAnimating()) screen.onButtonPressed(0);
	// Engage once per offer, as a player clicks once.
	local dialog = state.m.CombatDialog;
	if (dialog == null || !dialog.isVisible()) ::__autoplayEngaged = false;
	else if (::__autoplayEngage && !::__autoplayEngaged && !dialog.isAnimating())
	{
		::__autoplayEngaged = true;
		dialog.onEngageButtonPressed();
	}
};
"#;

/// Put the player next to the nearest hostile party and offer its battle; the
/// autoplay (installed by [`unpause_world`]) engages, after which the loading screen
/// shows and the world state starts the combat and hides itself. Run frames to fight.
/// The world runs past the origin's intro first (see [`run_past_intro`]).
pub fn engage_nearest_hostile(engine: &GameEngine) -> Result<(), String> {
	run_past_intro(engine)?;
	offer_nearest_battle(engine)
}

/// [`engage_nearest_hostile`] without waiting for the origin's intro: for a loaded
/// campaign, whose intro is long past.
pub fn offer_nearest_battle(engine: &GameEngine) -> Result<(), String> {
	engine.run_script(ENGAGE_NEAREST_HOSTILE, "engage")
}

const ENGAGE_NEAREST_HOSTILE: &str = r#"
local player = ::World.State.getPlayer();
local target = null;
local best = 0;
foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 100000.0))
{
	if (!e.isParty() || e.getID() == player.getID() || e.isAlliedWithPlayer() || e.getTroops().len() == 0) continue;
	local d = e.getDistanceTo(player);
	if (target == null || d < best) { target = e; best = d; }
}
if (target == null) throw "no hostile party to fight";
::logInfo("combat target: " + target.getName() + " (" + target.getTroops().len() + " troops)");
local tp = target.getPos();
player.setPos(::createVec(tp.X - 30, tp.Y));
::World.setPlayerPos(player.getPos());
::__autoplayEngage = true;
::World.State.showCombatDialog(true, true, true, null, player.getPos());
"#;

/// One frame of a headless game with real time `real_delta_ms`: the engine's frame,
/// the headless UI client's replies to what the scripts showed, then the scripted
/// player (installed by [`unpause_world`]).
pub fn run_frame(engine: &GameEngine, real_delta_ms: f64) -> Result<(), String> {
	engine.frame(real_delta_ms)?;
	engine.pump_ui().map_err(|e| format!("ui client: {e}"))?;
	let autoplay = squirrel::SqTable::root(engine.vm()).get::<squirrel::SqStrongRef>("__autoplay");
	autoplay.map_or(Ok(()), |f| f.call(engine.vm(), ())).map_err(|e| format!("autoplay: {e}"))
}

#[cfg(test)]
mod mod_archive_tests {
	use super::*;

	#[test]
	fn extra_mods_join_the_load_order_by_name() {
		let installed = ["/g/data/mod_modern_hooks.zip", "/g/data/mod_msu.zip", "/g/data/mod_x.zip"];
		let extra = ["/dev/out/mod_msu.zip", "/dev/out/mod_a.zip", "/dev/out/zz.zip"];
		let paths = |p: &[&str]| p.iter().map(PathBuf::from).collect::<Vec<_>>();
		let mods = with_extra_mods(paths(&installed), paths(&extra)).unwrap();
		let want = ["/dev/out/mod_a.zip", "/g/data/mod_modern_hooks.zip", "/dev/out/mod_msu.zip", "/g/data/mod_x.zip", "/dev/out/zz.zip"];
		assert_eq!(mods, paths(&want));
		assert!(with_extra_mods(vec![], paths(&["/a/m.zip", "/b/m.zip"])).is_err());
	}
}
