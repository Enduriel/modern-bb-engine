//! Drive a real battle from the campaign map, headless: boot a new campaign (see
//! `boot_world`), let it run briefly, move the player next to a hostile party and start
//! the fight through the world state's own flow (combat dialog → engage → loading
//! screen → `TacticalFromWorldState`), then run frames. Reports how far it gets: an
//! error names the next missing native.
//!
//! Run: `cargo run -p bb-engine --example boot_combat -- [data-dir] [--frames N] [--fight]`
//! (`--fight`: the AI fights for the player's units too; `--trace FILE`: run FILE once
//! the campaign starts and offer the battle through its `::__combatTrace()`, from the
//! paused world, for RNG traces against the original).

use std::path::PathBuf;

use bb_engine::harness;

/// Where the battle stands: round, whose turn, units per faction.
const BATTLE_STATUS: &str = r#"
if (!("Entities" in ::Tactical) || ::Tactical.Entities == null) { ::logInfo("battle: not started yet"); return; }
local st = ::Tactical.State;
local s = "round " + ::Time.getRound() + ", finished " + ::Tactical.Entities.isCombatFinished() + ", ended " + (st != null && st.isBattleEnded()) + ", result " + ::Tactical.Entities.getCombatResult();
if (st != null) s += ", resultScreen " + st.m.TacticalCombatResultScreen.isVisible() + ", finishable " + st.m.IsGameFinishable;
local active = ::Tactical.TurnSequenceBar.getActiveEntity();
s += ", active " + (active == null ? "none" : active.getName() + " (faction " + active.getFaction() + ", AP " + active.getActionPoints() + ")");
foreach (f, list in ::Tactical.Entities.getAllInstances()) if (list.len() != 0) s += ", f" + f + "=" + list.len();
if (active != null && active.getAIAgent() != null)
{
	local a = active.getAIAgent();
	s += " | agent evaluating=" + a.isEvaluating() + " finished=" + a.isFinished() + " turnStarted=" + a.m.IsTurnStarted + " nextEval=" + a.m.NextEvaluationTime + " vtime=" + ::Time.getVirtualTime() + " camDelayed=" + ::Tactical.CameraDirector.isDelayed() + " aiPaused=" + ::Tactical.State.m.IsAIPaused + " gamePaused=" + ::Tactical.State.m.IsGamePaused + " behavior=" + (a.m.ActiveBehavior == null ? "null" : a.m.ActiveBehavior.getID());
}
::logInfo("battle: " + s);
"#;

fn main() {
	let mut dir = PathBuf::from(harness::STEAM_DATA_DIR);
	let mut frames = 600;
	let mut fight = false;
	let mut trace: Option<String> = None;
	let mut it = std::env::args().skip(1);
	while let Some(a) = it.next() {
		match a.as_str() {
			"--frames" => frames = it.next().and_then(|n| n.parse().ok()).expect("--frames N"),
			"--fight" => fight = true,
			// RNG-trace mode: run this script after the campaign starts, then call its
			// `::__combatTrace()` (which offers the battle) from the paused world.
			"--trace" => trace = Some(it.next().expect("--trace FILE")),
			d => dir = PathBuf::from(d),
		}
	}
	let mut engine = harness::open_install(&dir).expect("open install");
	engine.vm_mut().set_print_handler(|s| eprint!("{s}"));
	engine.vm_mut().install_standard_error_handlers();

	let result = harness::boot_menu(&engine)
		.and_then(|()| harness::prepare_new_campaign(&engine, "rust-engine-test"))
		.and_then(|()| harness::start_new_campaign(&engine))
		.and_then(|()| match &trace {
			Some(file) => {
				let script = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
				harness::install_autoplay(&engine)?;
				engine.run_script(&script, "trace")?;
				// As the original probe does: ~2 s on the paused world first (the scenario's
				// intro event fires after 1 s of real time).
				(0..120).try_for_each(|_| harness::run_frame(&engine, 1000.0 / 60.0))?;
				engine.run_script("::__autoplayEngage = true; ::__combatTrace();", "trace")
			}
			None => {
				harness::unpause_world(&engine)?;
				(0..60).try_for_each(|_| harness::run_frame(&engine, 1000.0 / 60.0))?;
				if fight {
					engine.run_script("::__autoplayFight = true;", "fight")?;
				}
				harness::engage_nearest_hostile(&engine)
			}
		})
		.and_then(|()| {
			(0..frames).try_for_each(|n| {
				harness::run_frame(&engine, 1000.0 / 60.0).map_err(|e| format!("frame {n}: {e}"))?;
				if n % 150 == 0 || n + 1 == frames {
					engine.run_script(BATTLE_STATUS, "battle_status")?;
				}
				Ok(())
			})
		});
	println!("states: {:?}", engine.root_state().state_names());
	match result {
		Ok(()) => println!("✅ ran {frames} frames after starting the battle"),
		Err(e) => println!("⏹  stopped at the next gap:\n   {e}"),
	}
}
