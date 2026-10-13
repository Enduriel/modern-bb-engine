//! Drive the REAL new-game flow into a running (paused) campaign world, headless:
//! boot the menu, then add `WorldState` (its `onInit` stands up the 11 world managers),
//! and run the genuine `startNewCampaign` (worldgen + `Assets.init` + faction creation +
//! the starting scenario spawning the player). UI init is stubbed (logic-first) and the
//! auto-`init()` neutered so we drive the sequence explicitly. Reports how far the world
//! boot gets before hitting an unimplemented World native — the next-gap signal for the
//! running-world tier (analogous to `boot_worldgen`).
//!
//! Run: `cargo run -p bb-engine --example boot_world -- [data-dir]`

use std::path::PathBuf;

use bb_engine::DatScriptSource;
use squirrel_bb::Engine;

const DEFAULT_DATA: &str = concat!(env!("HOME"), "/.local/share/Steam/steamapps/common/Battle Brothers/data");

fn main() {
	let dir = PathBuf::from(
		std::env::args()
			.nth(1)
			.unwrap_or_else(|| DEFAULT_DATA.to_string()),
	);
	let mut dats: Vec<PathBuf> = std::fs::read_dir(&dir)
		.expect("read data dir")
		.flatten()
		.map(|e| e.path())
		.filter(|p| {
			let n = p
				.file_name()
				.map(|n| n.to_string_lossy().into_owned())
				.unwrap_or_default();
			n.starts_with("data_") && n.ends_with(".dat")
		})
		.collect();
	dats.sort();
	let source = DatScriptSource::open(&dats).expect("open archives");

	let mut engine = Engine::new(source);
	engine.vm_mut().set_print_handler(|s| eprint!("{s}"));
	engine.vm_mut().install_standard_error_handlers();

	engine
		.run_script(
			r#"
			::Const <- {};
			local function inc(f) { try { ::include(f); } catch(e){} }
			inc("scripts/tools/weak_table_ref");
			foreach (g in ["scripts/config/global","scripts/config/ui","scripts/config/strings",
			               "scripts/config/character_names","scripts/config/spawnlist_master"]) inc(g);
			foreach (f in ::IO.enumerateFiles("scripts/config/")) inc(f);
			local dlc = ::IO.enumerateFiles("scripts/dlc/");
			if (dlc != null) foreach (f in dlc) inc(f);
			"#,
			"config",
		)
		.expect("config");
	engine.boot("scripts/root_state").expect("boot menu");
	println!("menu booted; driving new-game world boot…");

	// Drive the genuine new-game sequence. Errors name the next missing World native.
	let result = engine.run_script(
		r#"
		// Stub the world UI init + neuter the auto-init so we drive the new-game
		// sequence explicitly (logic-first; the ~30 UI screens are deferred to later).
		// The stub gives each UI ref world_state's LOGIC touches a generic no-op object
		// (a table whose _get returns a no-op closure), so e.g. setPause's
		// World.TopbarDayTimeModule.hideMessage() is harmless.
		this.include("scripts/states/world_state");
		this.world_state.onInitUI = function() {
			local function uiStub() {
				local s = {};
				s.setdelegate({ _get = function( _k ) { return function( ... ) {}; } });
				return s;
			}
			foreach (n in ["MenuStack","WorldScreen","WorldTownScreen","EventScreen","CombatDialog",
			               "WorldEventPopupScreen","RelationsScreen","ObituaryScreen","CampfireScreen",
			               "WorldMenuScreen","WorldGameFinishScreen","CharacterScreen"])
				this.m.rawset(n, uiStub());
			this.World.rawset("TopbarDayTimeModule", uiStub());
		};
		this.world_state.init = function() {};

		// Add the world state as a RootState child: its onInit stands up the 11
		// managers (EntityManager/FactionManager/…), the stubbed onInitUI, the neutered
		// init, and sets World.State + the manager refs.
		this.RootState.add("WorldState", "scripts/states/world_state");
		local ws = this.RootState.get("WorldState");

		// Build new-campaign settings with the real starting scenario, then run the
		// genuine generation (worldgen → Assets.init → createFactions → scenario spawn).
		local settings = {
			Name = "Boot Test Company",
			Banner = "banner_04",
			Difficulty = 1,
			EconomicDifficulty = 1,
			BudgetDifficulty = 1,
			Ironman = false,
			PermanentDestruction = false,
			ExplorationMode = false,
			GreaterEvil = this.Const.World.GreaterEvilType.Random,
			Seed = "rust-engine-test",
			StartingScenario = this.new("scripts/scenarios/world/early_access_scenario")
		};
		ws.setNewCampaignSettings(settings);
		ws.startNewCampaign();
		::genOk <- true;
		"#,
		"world_boot",
	);
	match result {
		Ok(()) => {
			println!("✅ new-game world boot ran to completion");
			let (w, h) = engine.world_grid().dims();
			println!("   grid {w}x{h}; entities spawned = {}", engine.world_entities().count());
			let _ = engine.run_script(
				r#"
				this.logInfo("settlements = " + this.World.EntityManager.getSettlements().len());
				this.logInfo("player = " + (this.World.State.m.Player != null ? "set" : "null"));
				"#,
				"diag",
			);
		}
		Err(e) => println!("⏹  world boot stopped at the next gap:\n   {e}"),
	}
}
