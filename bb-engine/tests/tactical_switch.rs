//! `switchEntities` and `teleport`'s `onDone` in and out of the player's sight, as the
//! original's (`0x4ca160`, `0x4ca7a0`, probed 2026-10-20). Out of sight a switch is
//! instant and calls `onDone(tag)` once, so a callback of `(unit, tag)`, the arguments
//! each landing hands it in sight, fails there; that error goes to the error handler,
//! not to the caller, and the units still swap. Mods depend on this quirk (Reforged
//! passes null and calls its callback from `onMovementFinish`).
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

/// Two enemies, tiles in and out of the player's sight, a log of their moves and the
/// callbacks.
const SETUP: &str = r#"
	// The AI would move units in between.
	::Tactical.State.m.IsAIPaused = true;
	::__log <- [];
	::__e <- [];
	foreach (e in ::Tactical.Entities.getAllInstancesAsArray())
		if (!e.isAlliedWithPlayer() && e.isPlacedOnMap()) ::__e.push(e);
	::__fog <- [];
	::__seen <- [];
	local size = ::Tactical.getMapSize();
	for (local x = 0; x < size.X; x++)
		for (local y = 0; y < size.Y; y++)
		{
			local t = ::Tactical.getTileSquare(x, y);
			if (!t.IsEmpty) continue;
			if (t.IsVisibleForPlayer) ::__seen.push(t); else ::__fog.push(t);
		}
	foreach (e in [::__e[0], ::__e[1]])
	{
		local finish = e.onMovementFinish;
		e.onMovementFinish <- function ( _tile ) { ::__log.push("finish " + this.getID()); return finish(_tile); };
	}
	::__d <- function ( _v ) {
		if (typeof _v == "table" || typeof _v == "instance")
		{
			try { return "unit" + _v.getID(); } catch (e) {}
			return typeof _v;
		}
		return _v + "";
	};
	// `this` is the navigator, as Sqrat's callbacks have it.
	::__cbVar <- function ( ... ) {
		local s = this == ::Tactical.getNavigator() ? "cbVar" : "cbVar(this not the navigator)";
		foreach (a in vargv) s += " " + ::__d(a);
		::__log.push(s);
	};
	::__cb1 <- function ( _a ) { ::__log.push("cb1 " + ::__d(_a)); };
	::__cb2 <- function ( _a, _b ) { ::__log.push("cb2 " + ::__d(_a) + " " + ::__d(_b)); };
	::__place <- function ( _e, _tile ) {
		if (!::Tactical.getNavigator().teleport(_e, _tile, null, null, false, 0.0)) throw "placing failed";
	};
	::__a <- ::__e[0].getID();
	::__b <- ::__e[1].getID();
"#;

/// A battle with two enemies on the map; `None` without the Steam install.
fn battle() -> Option<harness::GameEngine> {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return None;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::unpause_world(&engine).expect("unpause");
	for frame in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
	}
	harness::engage_nearest_hostile(&engine).expect("engage");
	let in_battle = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible() && ::Tactical.Entities.getAllInstancesAsArray().filter(@(i, e) !e.isAlliedWithPlayer()).len() > 1";
	for _ in 0..3_000 {
		if player::eval_i64(&engine, &format!("({in_battle}) ? 1 : 0")).unwrap() == Some(1) {
			break;
		}
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	engine.run_script(SETUP, "setup").expect("two enemies and tiles");
	Some(engine)
}

/// Run `call` (an expression), then frames until nothing travels; what it answered (or
/// threw) and the log, with the units' ids as `a` and `b`.
fn act(engine: &harness::GameEngine, call: &str) -> (String, String) {
	let answer = player::eval_string(
		engine,
		&format!("(function() {{ ::__log = []; try {{ return \"\" + ({call}); }} catch (e) {{ return \"threw \" + e; }} }})()"),
	)
	.unwrap()
	.unwrap();
	for _ in 0..600 {
		if player::eval_i64(engine, "::Tactical.getNavigator().IsTravelling ? 1 : 0").unwrap() == Some(0) {
			break;
		}
		harness::run_frame(engine, 1000.0 / 60.0).unwrap();
	}
	let a = player::eval_i64(engine, "::__a").unwrap().unwrap();
	let b = player::eval_i64(engine, "::__b").unwrap().unwrap();
	let log = player::eval_string(engine, "(function() { local s = \"\"; foreach (i, l in ::__log) s += (i == 0 ? \"\" : \"; \") + l; return s; })()")
		.unwrap()
		.unwrap_or_default();
	let log = log.replace(&format!("unit{a}"), "a").replace(&format!("finish {a}"), "finish a");
	let log = log.replace(&format!("unit{b}"), "b").replace(&format!("finish {b}"), "finish b");
	(answer, log)
}

/// Where units `a` and `b` stand: `"x,y x,y"`.
fn tiles(engine: &harness::GameEngine) -> String {
	let at = |i: usize| format!("::__e[{i}].getTile().X + \",\" + ::__e[{i}].getTile().Y");
	player::eval_string(engine, &format!("{} + \" \" + {}", at(0), at(1))).unwrap().unwrap()
}

#[test]
fn switch_out_of_sight_hands_on_done_only_the_tag() {
	let Some(engine) = battle() else { return };
	engine.run_script("::__place(::__e[0], ::__fog[0]); ::__place(::__e[1], ::__fog[1]);", "fog").unwrap();

	// Instant: both finish, then onDone(tag), once.
	let before = tiles(&engine);
	let (answer, log) = act(&engine, "::Tactical.getNavigator().switchEntities(::__e[0], ::__e[1], ::__cbVar, \"tag\", 1.0)");
	assert_eq!(answer, "true");
	assert_eq!(log, "finish a; finish b; cbVar tag");
	let swapped = tiles(&engine);
	let flip = |s: &str| s.split(' ').rev().collect::<Vec<_>>().join(" ");
	assert_eq!(swapped, flip(&before), "swapped");

	// A callback of (unit, tag) fails out of the caller's sight: the switch still
	// happens and answers true.
	let (answer, log) = act(&engine, "::Tactical.getNavigator().switchEntities(::__e[0], ::__e[1], ::__cb2, \"tag\", 1.0)");
	assert_eq!(answer, "true");
	assert_eq!(log, "finish a; finish b");
	assert_eq!(tiles(&engine), before, "swapped back");

	let (answer, log) = act(&engine, "::Tactical.getNavigator().switchEntities(::__e[0], ::__e[1], ::__cb1, \"tag\", 1.0)");
	assert_eq!(answer, "true");
	assert_eq!(log, "finish a; finish b; cb1 tag");
}

#[test]
fn switch_in_sight_hands_each_landing_the_unit_and_tag() {
	let Some(engine) = battle() else { return };
	engine.run_script("::__place(::__e[0], ::__seen[0]); ::__place(::__e[1], ::__seen[1]);", "seen").unwrap();

	// Which lands first depends on the tiles' heights; each landing calls back after its
	// unit's onMovementFinish.
	let (answer, log) = act(&engine, "::Tactical.getNavigator().switchEntities(::__e[0], ::__e[1], ::__cb2, \"tag\", 1.0)");
	assert_eq!(answer, "true");
	assert_landings(&log, "cb2", "finish a; cb2 a tag; finish b; cb2 b tag");

	// A one-argument callback fails at each landing; the units land all the same.
	let (answer, log) = act(&engine, "::Tactical.getNavigator().switchEntities(::__e[0], ::__e[1], ::__cb1, \"tag\", 1.0)");
	assert_eq!(answer, "true");
	assert_landings(&log, "", "finish a; finish b");

	// One in sight is enough to fly.
	engine.run_script("::__place(::__e[1], ::__fog[0]);", "one in fog").unwrap();
	let (answer, log) = act(&engine, "::Tactical.getNavigator().switchEntities(::__e[0], ::__e[1], ::__cbVar, \"tag\", 1.0)");
	assert_eq!(answer, "true");
	assert_landings(&log, "cbVar", "finish a; cbVar a tag; finish b; cbVar b tag");
}

/// `log` is `expected`, or `expected` with its two landings (a's, then b's) the other way
/// round.
fn assert_landings(log: &str, callback: &str, expected: &str) {
	let swapped = if callback.is_empty() {
		"finish b; finish a".to_string()
	} else {
		format!("finish b; {callback} b tag; finish a; {callback} a tag")
	};
	assert!(log == expected || log == swapped, "{log:?}: expected {expected:?} (either landing first)");
}

#[test]
fn teleport_hands_on_done_the_unit_and_tag_in_and_out_of_sight() {
	let Some(engine) = battle() else { return };
	engine.run_script("::__place(::__e[0], ::__fog[0]);", "fog").unwrap();

	let (answer, log) = act(&engine, "::Tactical.getNavigator().teleport(::__e[0], ::__fog[1], ::__cbVar, \"tag\", false)");
	assert_eq!(answer, "true");
	assert_eq!(log, "finish a; cbVar a tag");

	// A callback of other arguments fails, out of sight and in it; the unit lands.
	let (answer, log) = act(&engine, "::Tactical.getNavigator().teleport(::__e[0], ::__fog[0], ::__cb1, \"tag\", false, 1.0)");
	assert_eq!(answer, "true");
	assert_eq!(log, "finish a");
	let (answer, log) = act(&engine, "::Tactical.getNavigator().teleport(::__e[0], ::__seen[0], ::__cb1, \"tag\", true, 1.0)");
	assert_eq!(answer, "true");
	assert_eq!(log, "finish a");
	assert_eq!(player::eval_i64(&engine, "::__e[0].getTile().isSameTileAs(::__seen[0]) ? 1 : 0").unwrap(), Some(1), "landed");

	let (answer, log) = act(&engine, "::Tactical.getNavigator().teleport(::__e[0], ::__seen[1], ::__cbVar, \"tag\", false, 0.0)");
	assert_eq!(answer, "true");
	assert_eq!(log, "finish a; cbVar a tag");
}
