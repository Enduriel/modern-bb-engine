//! Shared headless boot steps for the real game, used by the examples and the
//! integration tests: open an install's archives, load config and DLC, boot the menu,
//! and drive the genuine new-campaign sequence.

use std::io;
use std::path::{Path, PathBuf};

use squirrel_bb::{Engine, NativeConfig, installed_dlc};

use crate::assets::{load_fog_stamp, load_sprites};
use crate::mounts::Mounts;
use crate::{DatScriptSource, ScriptCache};

/// The data directory of a default Linux Steam install.
pub const STEAM_DATA_DIR: &str = "~/.local/share/Steam/steamapps/common/Battle Brothers/data";

/// Saves the original wrote, for the tests of loading and writing its format: its
/// campaigns as capture probes saved them, with its own dumps of their state (see the
/// README there). Not in the repository (each save is ~6.7 MB); tests skip without
/// them.
pub const ORIGINAL_SAVES_DIR: &str = "original-saves";

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
/// after the `data_*.dat` archives (by name; a later archive wins). The probe harness's
/// own zip (`mod_zz_bb_probe`, present only while a probe runs) is left out.
///
/// While a probe runs, the probe runner moves the installed mods (all but Modern Hooks) out
/// of `data/` into [`HELD_MODS`] beside it and puts them back afterwards; they count as
/// installed all the same, so tests see the installed mods whether or not a probe runs.
pub fn mod_archives(dir: &Path) -> io::Result<Vec<PathBuf>> {
	let zips_in = |d: &Path| -> io::Result<Vec<PathBuf>> {
		Ok(std::fs::read_dir(d)?
			.flatten()
			.map(|e| e.path())
			.filter(|p| {
				p.file_name().is_some_and(|n| {
					let n = n.to_string_lossy();
					n.ends_with(".zip") && !n.starts_with("mod_zz_bb_probe")
				})
			})
			.collect())
	};
	let mut zips = zips_in(dir)?;
	if let Some(held) = dir.parent().map(|g| g.join(HELD_MODS)).filter(|h| h.is_dir()) {
		for zip in zips_in(&held).unwrap_or_default() {
			if !zips.iter().any(|z| z.file_name() == zip.file_name()) {
				zips.push(zip);
			}
		}
	}
	zips.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
	Ok(zips)
}

/// Where a probe run keeps the installed mods, beside the install's `data/`.
pub const HELD_MODS: &str = "_bb_probe_held_mods";

/// Where a mod [`mod_archives`] listed is now: `path`, or the same zip moved by a probe
/// since (into [`HELD_MODS`], or back into `data/`). An opened archive survives the
/// move; this is for opening one by its listed path.
pub fn locate_mod(path: &Path) -> PathBuf {
	if path.exists() {
		return path.to_path_buf();
	}
	let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
		return path.to_path_buf();
	};
	let other = if dir.file_name().is_some_and(|n| n == HELD_MODS) { "data" } else { HELD_MODS };
	dir.parent()
		.map(|game| game.join(other).join(name))
		.filter(|moved| moved.exists())
		.unwrap_or_else(|| path.to_path_buf())
}

/// An engine over the install at `dir` with its mods mounted ([`mod_archives`]), as the
/// original loads them.
pub fn open_install_with_mods(dir: &Path) -> io::Result<GameEngine> {
	let mut archives = data_archives(dir)?;
	archives.extend(mod_archives(dir)?);
	open_mounted(dir, archives, |_| {})
}

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
	mut wait: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
	load_config(engine)?;
	engine.boot("scripts/root_state")?;
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
	engine.init_pending_states()
}

/// Run the genuine `startNewCampaign`: worldgen, assets, factions, the starting
/// scenario, and the faction simulation.
pub fn start_new_campaign(engine: &GameEngine) -> Result<(), String> {
	engine.run_script("::__bootWorldState.startNewCampaign();", "start")
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
/// the player's units too, handed over as a charm hands them (no longer player
/// controlled). A charmed unit that dies isn't taken off the player's roster, as in the
/// original; with `::__autoplayDieAsPlayers` set too, a handed-over unit is the
/// player's again as it dies, so its death is a brother's (off the roster, items
/// released), as when a player plays the battle. With `::__autoplayPlayer` set to a function, that function
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
pub fn engage_nearest_hostile(engine: &GameEngine) -> Result<(), String> {
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
	fn mods_a_probe_holds_still_count() {
		let game = std::env::temp_dir().join(format!("bb-held-mods-{}", std::process::id()));
		let (data, held) = (game.join("data"), game.join(HELD_MODS));
		std::fs::create_dir_all(&data).unwrap();
		std::fs::create_dir_all(&held).unwrap();
		for (d, name) in [(&data, "mod_modern_hooks.zip"), (&data, "mod_zz_bb_probe.zip"), (&held, "mod_msu.zip"), (&held, "a_mod.zip")] {
			std::fs::write(d.join(name), b"").unwrap();
		}
		let mods = mod_archives(&data).unwrap();
		let names: Vec<String> = mods.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
		assert_eq!(names, ["a_mod.zip", "mod_modern_hooks.zip", "mod_msu.zip"]);
		// The probe ends and puts MSU back: its listed path finds it in data/.
		let msu = mods.iter().find(|p| p.ends_with("mod_msu.zip")).unwrap().clone();
		std::fs::rename(held.join("mod_msu.zip"), data.join("mod_msu.zip")).unwrap();
		assert_eq!(locate_mod(&msu), data.join("mod_msu.zip"));
		std::fs::remove_dir_all(&game).unwrap();
	}
}
