//! An error thrown inside a script callback a native calls, as the original handles it
//! (probed 2026-10-21): it is reported (a `Script Error` row with its stack in log.html),
//! once per call, and nothing else happens. The native goes on (a tile query visits
//! every tile, a switch still swaps) and returns normally to its caller; the next
//! scheduled event still runs. A callback that can't even be called with the arguments
//! handed (a wrong parameter count) fails silently: Squirrel reports call-setup errors
//! only with no script on the stack.
//!
//! Covered: `Time.scheduleEvent` (Real and Virtual), `World.queryTilesInRange`,
//! `Tactical.queryTilesInRange` (both overloads), `Tactical.queryActorsInRange`,
//! `switchEntities` and `teleport` (in and out of sight), the battle camera's movement
//! callbacks and the save and load callbacks (the save is still written, the load
//! still happens).
//!
//! And the `this` each callback sees: the receiver of the native that took it, as
//! Sqrat keeps it (`Time`, `World`, `Tactical`, the camera; another table when the
//! native is called on one with `.call`), unless the function is bound.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

/// Each `__thrower(k)` counts its calls in `__n[k]` and throws `"BB_PROBE_ERR_" + k`;
/// `__try(k, f)` records what `f` answered or threw.
const HELPERS: &str = r#"
	::__n <- {};
	::__out <- {};
	::__count <- function ( _k ) { if (!(_k in ::__n)) ::__n[_k] <- 0; ::__n[_k]++; };
	::__thrower <- function ( _k ) { return function ( ... ) { ::__count(_k); throw "BB_PROBE_ERR_" + _k; }; };
	::__try <- function ( _k, _f ) {
		local r;
		try { r = "returned " + _f(); } catch (e) { r = "threw " + e; }
		::__out[_k] <- r;
	};
	::__calls <- @(_k) _k in ::__n ? ::__n[_k] : 0;
	::__env <- { Name = "env" };
	::__thisSeen <- {};
	::__who <- function ( _t ) {
		if (_t == ::getroottable()) return "root";
		if (_t == ::__env) return "env";
		foreach (k in ["World", "Tactical", "Time"]) if (_t == ::getroottable()[k]) return k;
		if (("Tactical" in ::getroottable()) && ::Tactical.isVisible() && _t == ::Tactical.getCamera()) return "camera";
		return typeof _t;
	};
	::__thisOf <- function ( _k ) { return function ( ... ) { ::__thisSeen[_k] <- ::__who(this); }; };
	::__this <- @(_k) _k in ::__thisSeen ? ::__thisSeen[_k] : "not called";
"#;

fn eval(engine: &harness::GameEngine, expr: &str) -> String {
	player::eval_string(engine, &format!("\"\" + ({expr})")).unwrap().unwrap_or_default()
}

fn frames(engine: &harness::GameEngine, n: usize) {
	for _ in 0..n {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap();
	}
}

/// How many times log.html reports `BB_PROBE_ERR_<k>` (as a whole word).
fn reported(log: &Path, k: &str) -> usize {
	let text = std::fs::read_to_string(log).unwrap_or_default();
	let needle = format!("BB_PROBE_ERR_{k}");
	text.match_indices(&needle)
		.filter(|(i, _)| !text[i + needle.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
		.count()
}

#[test]
fn errors_in_callbacks_are_reported_and_the_native_goes_on() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
		return;
	}
	let logs = std::env::temp_dir().join(format!("bb-callback-errors-{}", std::process::id()));
	std::fs::create_dir_all(&logs).unwrap();
	let log = logs.join("log.html");
	squirrel_bb::html_log::open(&log, "callback errors").unwrap();

	// A gliding battle camera, so a move starts and stops (an instant one never moves).
	let mut engine = harness::open_install_with(dir, |c| c.battle_camera = squirrel_bb::BattleCamera::Glides).expect("open install");
	squirrel_bb::report_script_errors(engine.vm_mut());
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::unpause_world(&engine).expect("unpause");
	frames(&engine, 60);
	engine.run_script(HELPERS, "helpers").unwrap();

	// The world: two Real events at the same time, the first throwing; the tile query.
	engine
		.run_script(
			"::Time.scheduleEvent(::TimeUnit.Real, 100, ::__thrower(\"w_real\"), null);
			 ::Time.scheduleEvent(::TimeUnit.Real, 100, function ( _ ) { ::__count(\"w_real_next\"); }, null);
			 ::__try(\"w_query\", @() ::World.queryTilesInRange(::World.State.getPlayer().getTile(), 1, 2, ::__thrower(\"w_query\"), null));
			 ::__try(\"w_query_arity\", @() ::World.queryTilesInRange(::World.State.getPlayer().getTile(), 1, 2, function ( _a ) { ::__count(\"w_query_arity\"); }, null));",
			"world",
		)
		.unwrap();
	frames(&engine, 30);
	assert_eq!(eval(&engine, "::__out.w_query"), "returned null", "the query returns normally");
	assert_eq!(eval(&engine, "::__calls(\"w_query\")"), "18", "every tile of rings 1 and 2");
	assert_eq!(reported(&log, "w_query"), 18, "each throw reported");
	assert_eq!(eval(&engine, "::__out.w_query_arity"), "returned null");
	assert_eq!(eval(&engine, "::__calls(\"w_query_arity\")"), "0", "never called");
	assert_eq!(reported(&log, "w_query_arity"), 0);
	assert_eq!(eval(&engine, "::__calls(\"w_real\") + \" \" + ::__calls(\"w_real_next\")"), "1 1", "the next event still runs");
	assert_eq!(reported(&log, "w_real"), 1);

	// `this`: the native's receiver, or the table it was called on, unless bound.
	engine
		.run_script(
			"::Time.scheduleEvent(::TimeUnit.Real, 10, ::__thisOf(\"event\"), null);
			 ::Time.scheduleEvent.call(::__env, ::TimeUnit.Real, 10, ::__thisOf(\"event_env\"), null);
			 local tile = ::World.State.getPlayer().getTile();
			 ::World.queryTilesInRange(tile, 1, 1, ::__thisOf(\"w_query\"), null);
			 ::World.queryTilesInRange.call(::__env, tile, 1, 1, ::__thisOf(\"w_query_env\"), null);
			 ::World.queryTilesInRange(tile, 1, 1, ::__thisOf(\"w_query_bound\").bindenv(::__env), null);",
			"this",
		)
		.unwrap();
	frames(&engine, 10);
	for (k, this) in [("event", "Time"), ("event_env", "env"), ("w_query", "World"), ("w_query_env", "env"), ("w_query_bound", "env")] {
		assert_eq!(eval(&engine, &format!("::__this(\"{k}\")")), this, "{k}'s this");
	}

	// Save and load callbacks that throw: reported, `this` is `World`, and the save is
	// written and loads.
	engine
		.run_script(
			r#"
			local st = ::World.State;
			::World.setOnBeforeSaveCallback(function ( _out ) { ::__thisSeen.before_save <- ::__who(this); st.onBeforeSerialize(_out); throw "BB_PROBE_ERR_before_save"; });
			::World.setOnSaveCallback(function ( _out ) { ::__thisSeen.save <- ::__who(this); st.onSerialize(_out); throw "BB_PROBE_ERR_save"; });
			::World.setOnBeforeLoadCallback(function ( _in ) { ::__thisSeen.before_load <- ::__who(this); st.onBeforeDeserialize(_in); throw "BB_PROBE_ERR_before_load"; });
			::World.setOnLoadCallback(function ( _in ) { ::__thisSeen.load <- ::__who(this); st.onDeserialize(_in); throw "BB_PROBE_ERR_load"; });
			::__try("save", @() st.saveCampaign("bb_probe_callback_errors"));
			::__saved <- false;
			foreach (s in ::PersistenceManager.queryStorages()) if (s.getFileName() == "bb_probe_callback_errors") ::__saved = true;
			st.m.CampaignLoadTime = -100.0;
			::__try("load", @() st.loadCampaign("bb_probe_callback_errors"));
			"#,
			"save and load",
		)
		.unwrap();
	assert_eq!(eval(&engine, "::__out.save"), "returned null", "the save returns normally");
	assert_eq!(eval(&engine, "::__saved"), "true", "the save is written");
	assert_eq!(eval(&engine, "::__out.load"), "returned null", "the load returns normally");
	for k in ["before_save", "save", "before_load", "load"] {
		assert_eq!(eval(&engine, &format!("::__this(\"{k}\")")), "World", "{k}'s this");
		assert_eq!(reported(&log, k), 1, "{k} reported");
	}
	engine
		.run_script(
			r#"
			local st = ::World.State;
			::World.setOnBeforeSaveCallback(st.onBeforeSerialize.bindenv(st));
			::World.setOnSaveCallback(st.onSerialize.bindenv(st));
			::World.setOnBeforeLoadCallback(st.onBeforeDeserialize.bindenv(st));
			::World.setOnLoadCallback(st.onDeserialize.bindenv(st));
			::PersistenceManager.deleteStorage("bb_probe_callback_errors");
			"#,
			"callbacks back",
		)
		.unwrap();
	frames(&engine, 30);

	// A battle.
	harness::engage_nearest_hostile(&engine).expect("engage");
	let in_battle = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible() && ::Tactical.Entities.getAllInstancesAsArray().filter(@(i, e) !e.isAlliedWithPlayer()).len() > 1";
	for _ in 0..3_000 {
		if player::eval_i64(&engine, &format!("({in_battle}) ? 1 : 0")).unwrap() == Some(1) {
			break;
		}
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	engine
		.run_script(
			r#"
			::Tactical.State.m.IsAIPaused = true;
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
			::__place <- function ( _e, _tile ) {
				if (!::Tactical.getNavigator().teleport(_e, _tile, null, null, false, 0.0)) throw "placing failed";
			};
			// Off the map's edge, so all 18 tiles of rings 1 and 2 exist.
			::__mid <- ::Tactical.getTileSquare(size.X / 2, size.Y / 2);
			"#,
			"battle",
		)
		.unwrap();

	// Virtual events.
	engine
		.run_script(
			"::Time.scheduleEvent(::TimeUnit.Virtual, 100, ::__thrower(\"t_virtual\"), null);
			 ::Time.scheduleEvent(::TimeUnit.Virtual, 100, function ( _ ) { ::__count(\"t_virtual_next\"); }, null);",
			"virtual",
		)
		.unwrap();
	frames(&engine, 30);
	assert_eq!(eval(&engine, "::__calls(\"t_virtual\") + \" \" + ::__calls(\"t_virtual_next\")"), "1 1");
	assert_eq!(reported(&log, "t_virtual"), 1);

	// The battle's queries.
	engine
		.run_script(
			"::__try(\"t_query_tiles\", @() ::Tactical.queryTilesInRange(::__mid, 1, 2, false, [], ::__thrower(\"t_query_tiles\"), null));
			 ::__try(\"t_query_tiles8\", @() ::Tactical.queryTilesInRange(::__mid, 1, 2, false, -9999, [], ::__thrower(\"t_query_tiles8\"), null));
			 ::__try(\"t_query_actors\", @() ::Tactical.queryActorsInRange(::__e[0].getTile(), 0, 60, ::__thrower(\"t_query_actors\"), null));
			 ::Tactical.queryTilesInRange(::__mid, 1, 1, false, [], ::__thisOf(\"t_query_tiles_this\"), null);
			 ::Tactical.queryActorsInRange(::__e[0].getTile(), 0, 60, ::__thisOf(\"t_query_actors_this\"), null);
			 ::__actors <- 0;
			 ::Tactical.queryActorsInRange(::__e[0].getTile(), 0, 60, function ( _a, _d ) { ::__actors++; }, null);",
			"queries",
		)
		.unwrap();
	for k in ["t_query_tiles", "t_query_tiles8", "t_query_actors"] {
		assert_eq!(eval(&engine, &format!("::__out.{k}")), "returned null", "{k} returns normally");
	}
	assert_eq!(eval(&engine, "::__calls(\"t_query_tiles\")"), "18");
	assert_eq!(eval(&engine, "::__calls(\"t_query_tiles8\")"), "18");
	assert_eq!(eval(&engine, "::__calls(\"t_query_actors\")"), eval(&engine, "::__actors"), "every actor");
	assert_eq!(eval(&engine, "::__this(\"t_query_tiles_this\")"), "Tactical");
	assert_eq!(eval(&engine, "::__this(\"t_query_actors_this\")"), "Tactical");
	assert_eq!(reported(&log, "t_query_tiles"), 18);
	assert_eq!(reported(&log, "t_query_tiles8"), 18);
	assert_eq!(reported(&log, "t_query_actors").to_string(), eval(&engine, "::__actors"));

	// Switches and teleports: reported, and they still go.
	let travel_done = |engine: &harness::GameEngine| {
		for _ in 0..600 {
			if eval(engine, "::Tactical.getNavigator().IsTravelling") == "false" {
				return;
			}
			harness::run_frame(engine, 1000.0 / 60.0).unwrap();
		}
		panic!("still travelling");
	};
	let cases = [
		("t_switch_fog", "::__place(::__e[0], ::__fog[0]); ::__place(::__e[1], ::__fog[1]);", "::Tactical.getNavigator().switchEntities(::__e[0], ::__e[1], ::__thrower(\"t_switch_fog\"), \"tag\", 1.0)", 1),
		("t_switch_seen", "::__place(::__e[0], ::__seen[0]); ::__place(::__e[1], ::__seen[1]);", "::Tactical.getNavigator().switchEntities(::__e[0], ::__e[1], ::__thrower(\"t_switch_seen\"), \"tag\", 1.0)", 2),
		("t_teleport_fog", "::__place(::__e[0], ::__fog[2]);", "::Tactical.getNavigator().teleport(::__e[0], ::__fog[3], ::__thrower(\"t_teleport_fog\"), \"tag\", false, 1.0)", 1),
		("t_teleport_seen", "::__place(::__e[0], ::__seen[2]);", "::Tactical.getNavigator().teleport(::__e[0], ::__seen[3], ::__thrower(\"t_teleport_seen\"), \"tag\", false, 1.0)", 1),
	];
	for (k, setup, call, calls) in cases {
		engine.run_script(&format!("{setup} ::__try(\"{k}\", @() {call});"), k).unwrap();
		travel_done(&engine);
		assert_eq!(eval(&engine, &format!("::__out.{k}")), "returned true", "{k}");
		assert_eq!(eval(&engine, &format!("::__calls(\"{k}\")")), calls.to_string(), "{k}");
		assert_eq!(reported(&log, k), calls, "{k} reported");
	}

	// The battle camera's movement callbacks, from a camera at rest.
	frames(&engine, 240);
	assert_eq!(eval(&engine, "::Tactical.getCamera().isMoving()"), "false", "at rest");
	engine
		.run_script(
			"::__tcam <- { start = ::__thrower(\"t_cam_start\"), end = ::__thrower(\"t_cam_end\") };
			 ::Tactical.getCamera().registerCallbacks(::__tcam.start, ::__tcam.end);
			 ::Tactical.getCamera().moveTo(::__e[::__e.len() - 1]);",
			"camera",
		)
		.unwrap();
	frames(&engine, 240);
	engine.run_script("::Tactical.getCamera().unregisterCallbacks(::__tcam.start, ::__tcam.end);", "camera off").unwrap();
	assert_eq!(eval(&engine, "::__calls(\"t_cam_start\") + \" \" + ::__calls(\"t_cam_end\")"), "1 1");
	assert_eq!(reported(&log, "t_cam_start"), 1);
	assert_eq!(reported(&log, "t_cam_end"), 1);
	engine
		.run_script(
			"::__tcam2 <- { start = ::__thisOf(\"cam_start\"), end = ::__thisOf(\"cam_end\") };
			 ::Tactical.getCamera().registerCallbacks(::__tcam2.start, ::__tcam2.end);
			 ::Tactical.getCamera().moveTo(::__e[0]);",
			"camera this",
		)
		.unwrap();
	frames(&engine, 240);
	engine.run_script("::Tactical.getCamera().unregisterCallbacks(::__tcam2.start, ::__tcam2.end);", "camera this off").unwrap();
	assert_eq!(eval(&engine, "::__this(\"cam_start\") + \" \" + ::__this(\"cam_end\")"), "camera camera");

	squirrel_bb::html_log::close();
	let _ = std::fs::remove_dir_all(&logs);
}
