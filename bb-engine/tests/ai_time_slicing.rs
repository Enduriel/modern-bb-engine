//! The AI thinks in slices, as the original's. Its evaluations check a time budget
//! (`isAllottedTimeReached`: more than `Const.AI.AllottedTimePerEvaluationRun` of
//! `Time.getExactTime()` since the slice began, and the background task suspended) and
//! yield there, going on where they were the next frame. The original runs the task on
//! a worker thread and suspends it when the next frame starts; ours runs it inline,
//! suspended from the start (`RootState::run_background`).
//!
//! Under the live exact clock (the windowed game) time passes within a slice, so the
//! budget runs out and an evaluation spreads over frames. Under the frame clock (tests,
//! traces) it doesn't, so evaluations never yield for time and battles stay
//! draw-exact. With an allotment of 0, every budget check the AI reaches under the live
//! clock yields (and it gets one loop's step further a frame). The yields are counted
//! through `Root.isBackgroundTaskSuspended`, which the check only asks once the
//! allotment is used up. With the vanilla allotment the AI still plays its turns.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};
use squirrel_bb::ExactClock;

const IN_TACTICAL: &str = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible() && ::Tactical.TurnSequenceBar.getActiveEntity() != null";

/// The frames the AI thinks with an allotment of 0.
const STARVED: u32 = 600;

/// The turns played after, with the vanilla allotment: brothers and bandits, all the AI's.
const TURNS: u32 = 6;

/// A battle against a few bandits, the AI fighting for the company: the budget yields in
/// `STARVED` frames with an allotment of 0, then (with the vanilla one) the budget
/// yields and frames until `TURNS` turns are over.
fn budget_yields(clock: ExactClock) -> (i64, i64, u32) {
	let mut engine = harness::open_install_with(Path::new(harness::STEAM_DATA_DIR), |c| c.exact_clock = clock).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::install_autoplay(&engine).expect("autoplay");
	// The scenario's intro event comes 1 s (real time) in; in a battle, its pause would
	// stop the battle's clock.
	harness::unpause_world(&engine).expect("unpause");
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	engine
		.run_script(
			r#"
			::__autoplayFight = true; ::__autoplayDieAsPlayers = true;
			::World.State.m.IsAutosaving = false;
			::__autoplayEngage = true;
			local st = ::World.State.get();
			local p = st.getLocalCombatProperties(st.getPlayer().getPos(), true);
			p.Entities = [];
			p.Parties = [];
			p.TerrainTemplate = "tactical.plains";
			p.CombatID = "Player";
			::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditRaiders, 40, ::World.FactionManager.getFactionOfType(::Const.FactionType.Bandits).getID());
			st.startScriptedCombat(p, true, true, true);
			"#,
			"battle",
		)
		.expect("battle");
	let mut frames = 0;
	while player::eval_i64(&engine, &format!("({IN_TACTICAL}) ? 1 : 0")).unwrap() != Some(1) {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
		frames += 1;
		assert!(frames < 6000, "the battle never began");
	}
	engine
		.run_script(
			r#"
			::__vanillaAllotment <- ::Const.AI.AllottedTimePerEvaluationRun;
			::Const.AI.AllottedTimePerEvaluationRun = 0.0;
			::__budgetYields <- 0;
			local suspended = ::Root.isBackgroundTaskSuspended;
			::Root.isBackgroundTaskSuspended = function()
			{
				local s = suspended.call(::Root);
				if (s) ::__budgetYields++;
				return s;
			};
			"#,
			"count the budget yields",
		)
		.expect("count");
	for _ in 0..STARVED {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	let starved = player::eval_i64(&engine, "::__budgetYields").unwrap().unwrap();
	engine
		.run_script("::Const.AI.AllottedTimePerEvaluationRun = ::__vanillaAllotment; ::__budgetYields = 0;", "vanilla allotment")
		.expect("vanilla allotment");
	let active = "(function() { local e = ::Tactical.TurnSequenceBar.getActiveEntity(); return e == null ? -1 : e.getID(); })()";
	let (mut turns, mut frames) = (0, 0);
	let mut last = player::eval_i64(&engine, active).unwrap();
	while turns < TURNS {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
		frames += 1;
		assert!(frames < 30_000, "{turns} turns in {frames} frames");
		let now = player::eval_i64(&engine, active).unwrap();
		if now != last && now != Some(-1) {
			turns += 1;
			last = now;
		}
	}
	(starved, player::eval_i64(&engine, "::__budgetYields").unwrap().unwrap(), frames)
}

#[test]
fn the_ai_yields_for_time_only_under_the_live_clock() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		eprintln!("skipping: no Steam install");
		return;
	}
	let (live, live_vanilla, live_frames) = budget_yields(ExactClock::Live);
	let (frame, frame_vanilla, frame_frames) = budget_yields(ExactClock::Frame);
	eprintln!(
		"allotment 0, {STARVED} frames: {live} budget yields under the live clock, {frame} under the frame clock; \
		 then {TURNS} turns: live {live_vanilla} yields in {live_frames} frames, frame {frame_vanilla} in {frame_frames}"
	);
	assert!(live > 0, "under the live clock the AI's evaluations yield for time");
	assert_eq!((frame, frame_vanilla), (0, 0), "under the frame clock they never do");
}
