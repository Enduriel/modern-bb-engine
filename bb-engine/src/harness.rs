//! Shared headless boot steps for the real game, used by the examples and the
//! integration tests: open an install's archives, load config and DLC, boot the menu,
//! and drive the genuine new-campaign sequence.

use std::io;
use std::path::{Path, PathBuf};

use squirrel_bb::{Engine, NativeConfig, installed_dlc};

use crate::DatScriptSource;

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
/// `scripts/dlc/` init script).
pub fn open_install(dir: &Path) -> io::Result<GameEngine> {
	let source = DatScriptSource::open(data_archives(dir)?)?;
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
	squirrel_bb::ui_client::pump(engine.ui(), engine.vm()).map(|_| ())
}

/// Add the world state and give it new-campaign settings for `seed`, ready for
/// [`start_new_campaign`]. The world UI is the real script UI, talking to the headless
/// UI bridge; the automatic `init()` is disabled so the caller drives the sequence.
/// The state is left in `::__bootWorldState`.
pub fn prepare_new_campaign(engine: &GameEngine, seed: &str) -> Result<(), String> {
	let script = r#"
		this.include("scripts/states/world_state");
		this.world_state.init = function() {};

		// onInit stands up the 11 managers, the UI and the disabled init.
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
	engine.run_script(&script, "world_boot")
}

/// Run the genuine `startNewCampaign`: worldgen, assets, factions, the starting
/// scenario, and the faction simulation.
pub fn start_new_campaign(engine: &GameEngine) -> Result<(), String> {
	engine.run_script("::__bootWorldState.startNewCampaign();", "start")
}

/// Let the world run, as a player does after starting a campaign: unpause, and from
/// then on let [`run_frame`]'s scripted player answer events.
pub fn unpause_world(engine: &GameEngine) -> Result<(), String> {
	engine.run_script(AUTOPLAY, "autoplay")?;
	engine.run_script(
		"::__bootWorldState.setAutoPause(false); ::__bootWorldState.setPause(false);",
		"unpause",
	)
}

/// The scripted player: picks the first option of whatever event or contract screen
/// is open, as clicking its first button would.
const AUTOPLAY: &str = r#"
::__autoplay <- function()
{
	if (!("State" in ::World) || ::World.State == null) return;
	local screen = ::World.State.getEventScreen();
	if (screen != null && screen.isVisible() && !screen.isAnimating()) screen.onButtonPressed(0);
};
"#;

/// One frame of a headless game with real time `real_delta_ms`: the engine's frame,
/// the headless UI client's replies to what the scripts showed, then the scripted
/// player (installed by [`unpause_world`]).
pub fn run_frame(engine: &GameEngine, real_delta_ms: f64) -> Result<(), String> {
	engine.frame(real_delta_ms)?;
	squirrel_bb::ui_client::pump(engine.ui(), engine.vm()).map_err(|e| format!("ui client: {e}"))?;
	let autoplay = squirrel::SqTable::root(engine.vm()).get::<squirrel::SqStrongRef>("__autoplay");
	autoplay.map_or(Ok(()), |f| f.call(engine.vm(), ())).map_err(|e| format!("autoplay: {e}"))
}
