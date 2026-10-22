//! Quitting a campaign while an event screen is open: the window tests' flake (bb ended
//! with "the index 'updateAssetsInformation' does not exist").
//!
//! `exitGame()` only shows the loading screen; the world goes on updating until the
//! page reports it animating (2 frames later in the original, probed 2026-10-22), and
//! an event can open in between (the event manager waits only for a visible or
//! animating loading screen). The event screen's backstep can't be popped by the world's
//! `onHide` (its validator always says no), so it is still on the menu stack when the
//! world finishes. `onDestroyUI` destroys the world screen first and the menu stack
//! after: popping the backstep then calls `updateTopbarAssets` on a world screen without
//! its topbar module, and the scripts throw. A vanilla script bug: the original throws
//! the same error, with the same stack (probed 2026-10-22). There it is logged, the rest
//! of `onFinish` is skipped (the old world state keeps its menu stack and world screen),
//! the world state is gone and the main menu shows; the game goes on.
//!
//! Here the event is fired at once after `exitGame()`, as the event manager would in
//! those frames, so the sequence is the same every run.
//!
//! Skips without the Steam install.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use bb_engine::{harness, tactical_player as player};

const AT_MENU: &str = "::RootState.get(\"WorldState\") == null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible()";
const ERROR: &str = "the index 'updateAssetsInformation' does not exist";

fn eval(engine: &harness::GameEngine, expr: &str) -> String {
	player::eval_string(engine, &format!("\"\" + ({expr})")).unwrap().unwrap_or_default()
}

/// A campaign's world, up and out of its loading screen, its intro event closed, as
/// `bb --new-campaign` starts one; script errors collected in the returned list.
fn campaign(script_errors: squirrel_bb::ScriptErrors) -> Option<(harness::GameEngine, Rc<RefCell<Vec<String>>>)> {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		harness::skip(format!("{} not present", dir.display()));
		return None;
	}
	let mut engine = harness::open_install(dir).expect("open install");
	let errors: Rc<RefCell<Vec<String>>> = Rc::default();
	let sink = errors.clone();
	engine.vm_mut().set_runtime_error_handler(move |e| sink.borrow_mut().push(squirrel_bb::terminal_text(e)));
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	engine.run_script("::RootState.get(\"MainMenuState\").hide();", "hide the menu").unwrap();
	engine.run_script("::World.State.m.MenuStack.popAll(true);", "close the intro").unwrap();
	engine.set_script_errors(script_errors);
	for _ in 0..600 {
		if eval(&engine, "::World.State.isInLoadingScreen()") == "false" {
			break;
		}
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	assert_eq!(eval(&engine, "::World.State.isInLoadingScreen()"), "false", "the world is up");
	harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	Some((engine, errors))
}

/// The in-game menu's Quit, as the window tests do it, then an event, as one fires in the
/// frames before the loading screen animates.
fn quit_with_an_event(engine: &harness::GameEngine) {
	engine
		.run_script(
			"::__oldWorld <- ::World.State; ::World.State.m.MenuStack.popAll(true); ::World.State.exitGame();
			 ::__fired <- ::World.Events.fire(\"event.early_access_scenario_intro\");",
			"quit, then an event",
		)
		.unwrap();
	assert_eq!(eval(engine, "::__fired"), "true", "the event screen opened");
	assert_eq!(eval(engine, "::__oldWorld.m.MenuStack.hasBacksteps()"), "true", "its backstep is on the menu stack");
}

#[test]
fn the_scripts_error_ends_a_frame_that_fails_on_script_errors() {
	let Some((engine, errors)) = campaign(squirrel_bb::ScriptErrors::Fail) else { return };
	quit_with_an_event(&engine);
	let failed = (0..600).find_map(|_| harness::run_frame(&engine, 1000.0 / 60.0).err());
	let failed = failed.expect("a frame fails");
	assert!(failed.contains(ERROR), "{failed}");
	assert_eq!(errors.borrow().len(), 1, "{:?}", errors.borrow());
}

#[test]
fn the_scripts_error_is_reported_and_the_game_goes_on_to_the_menu() {
	let Some((engine, errors)) = campaign(squirrel_bb::ScriptErrors::GoOn) else { return };
	quit_with_an_event(&engine);
	let mut at_menu = false;
	for _ in 0..600 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("every frame goes on");
		if eval(&engine, AT_MENU) == "true" {
			at_menu = true;
			break;
		}
	}
	assert!(at_menu, "the world state finished and the main menu shows");
	{
		let errors = errors.borrow();
		assert_eq!(errors.len(), 1, "reported once: {errors:?}");
		let report = &errors[0];
		assert!(report.starts_with(&format!("Script Error: {ERROR}")), "{report}");
		for function in ["updateTopbarAssets", "pop ", "popAll", "onDestroyUI", "onFinish"] {
			assert!(report.contains(function), "{function} on the stack, as in the original's:\n{report}");
		}
	}
	// As the original's: the rest of onDestroyUI never ran.
	assert_eq!(eval(&engine, "::__oldWorld.m.MenuStack != null"), "true", "the old world keeps its menu stack");
	assert_eq!(eval(&engine, "::__oldWorld.m.WorldScreen != null"), "true", "and its world screen");
	assert_eq!(eval(&engine, "::World.State == null"), "true");
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("the menu runs on");
	}
	assert_eq!(eval(&engine, AT_MENU), "true");
	assert_eq!(errors.borrow().len(), 1, "nothing else went wrong");
}
