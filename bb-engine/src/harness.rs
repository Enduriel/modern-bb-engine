//! Shared headless boot steps for the real game, used by the examples and the
//! integration tests: open an install's archives, load config and DLC, boot the menu,
//! and drive the genuine new-campaign sequence.

use std::io;
use std::path::{Path, PathBuf};

use squirrel_bb::{Engine, NativeConfig, installed_dlc};

use crate::{DatScriptSource, ScriptCache};

/// The data directory of a default Linux Steam install.
pub const STEAM_DATA_DIR: &str = "~/.local/share/Steam/steamapps/common/Battle Brothers/data";

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

/// An engine over the install at `dir`, configured as the original sees a Steam install:
/// DLC ownership is whichever DLC archives Steam installed (each ships its own
/// `scripts/dlc/` init script). Transcoded scripts are cached on disk as
/// [`ScriptCache::from_env`] configures (`BB_SCRIPT_CACHE=off` disables it).
pub fn open_install(dir: &Path) -> io::Result<GameEngine> {
	let source = DatScriptSource::open(data_archives(dir)?)?.with_cache(ScriptCache::from_env());
	let config = NativeConfig {
		is_steam_build: true,
		owned_dlc: installed_dlc(&source),
		..NativeConfig::default()
	};
	Ok(Engine::with_config(source, &config))
}

/// Run the game's own `scripts/preload`, as the original does before the menu: config
/// tables, DLC init scripts, then every script under `scripts/` (some define globals at
/// file level, e.g. the contract negotiation templates).
pub fn load_config(engine: &GameEngine) -> Result<(), String> {
	engine.run_script("::include(\"scripts/preload\");", "preload")
}

/// Boot to the main menu: config, then `scripts/root_state`. Then park the cursor, as
/// the first mouse event would, so frame code that reads it has a position (over the
/// UI, see `UI.setCursorPosition`).
pub fn boot_menu(engine: &GameEngine) -> Result<(), String> {
	load_config(engine)?;
	engine.boot("scripts/root_state")?;
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
		this.include("scripts/states/world_state");
		this.world_state.init = function() {};

		// Its onInit (run once it has its settings, see below) stands up the 11 managers,
		// the UI and the disabled init.
		this.RootState.add("WorldState", "scripts/states/world_state");
		local ws = this.RootState.get("WorldState");
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
/// the player's units too.
const AUTOPLAY: &str = r#"
::__autoplayEngage <- false;
::__autoplayFight <- false;
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
				// Its turn may already be under way: tell the new agent, as the turn
				// start does.
				if (bro.isTurnStarted()) agent.onTurnStarted();
			}
		}
		if (!st.isBattleEnded())
		{
			local bar = ::Tactical.TurnSequenceBar;
			local active = bar.getActiveEntity();
			if (active != null && active.isPlayerControlled()) bar.onNextTurnButtonPressed();
		}
		else
		{
			local results = st.m.TacticalCombatResultScreen;
			if (results != null && results.isVisible() && !results.isAnimating())
			{
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
	local dialog = state.m.CombatDialog;
	if (::__autoplayEngage && dialog != null && dialog.isVisible() && !dialog.isAnimating()) dialog.onEngageButtonPressed();
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
