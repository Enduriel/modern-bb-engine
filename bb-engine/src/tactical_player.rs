//! A player's commands in a battle, issued the way the original's input produces them:
//! clicks on tiles go through the engine's mouse input into the tactical state's
//! `onMouseInput` (via `Tactical.screenToTile`), and the buttons and the skill bar call
//! the turn bar's JS→SQ handlers, as the page does. Tests use this to play the company's
//! turns themselves instead of ending them.
//!
//! A click centres the camera on the tile and clicks the middle of the screen, so it
//! lands on that tile whatever the camera's scale. Commands act on the unit whose turn
//! it is; check [`ready`] first (no move or skill under way, input not locked).
//!
//! Keys go through the states' key input. The codes are the engine's: letters run from
//! 11 ('A') to 36 ('Z'), as the tactical state's bindings show (WASD, QE, C/I for the
//! inventory). The inventory is the character screen; its item moves are the screen's
//! JS→SQ callbacks, as its page calls them.

use squirrel::SqTable;
use squirrel_bb::{KeyInput, MouseInput};

use crate::harness::GameEngine;

/// Evaluate a Squirrel expression into `::__playerResult`.
fn eval(engine: &GameEngine, expr: &str) -> Result<(), String> {
	engine.run_script(&format!("::__playerResult <- ({expr});"), "player")
}

fn eval_bool(engine: &GameEngine, expr: &str) -> Result<Option<bool>, String> {
	eval(engine, expr)?;
	Ok(SqTable::root(engine.vm()).get::<bool>("__playerResult"))
}

/// Evaluate an integer Squirrel expression.
pub fn eval_i64(engine: &GameEngine, expr: &str) -> Result<Option<i64>, String> {
	eval(engine, expr)?;
	Ok(SqTable::root(engine.vm()).get::<i64>("__playerResult"))
}

/// Evaluate a string Squirrel expression.
pub fn eval_string(engine: &GameEngine, expr: &str) -> Result<Option<String>, String> {
	eval(engine, expr)?;
	Ok(SqTable::root(engine.vm()).get::<String>("__playerResult"))
}

/// Whether it's a player unit's turn and the game takes input now.
pub fn ready(engine: &GameEngine) -> Result<bool, String> {
	let ready = eval_bool(
		engine,
		"(\"State\" in ::Tactical) && ::Tactical.State != null && !::Tactical.State.isBattleEnded() \
		 && ::Tactical.TurnSequenceBar.getActiveEntity() != null \
		 && ::Tactical.TurnSequenceBar.getActiveEntity().isPlayerControlled() \
		 && ::Tactical.TurnSequenceBar.getActiveEntity().isTurnStarted() \
		 && !::Tactical.State.isInputLocked() \
		 && !::Tactical.getNavigator().isTravelling(::Tactical.TurnSequenceBar.getActiveEntity()) \
		 && ::Tactical.State.m.CurrentActionState != ::Const.Tactical.ActionState.TravelPath \
		 && ::Tactical.State.m.CurrentActionState != ::Const.Tactical.ActionState.ExecuteSkill",
	)?;
	Ok(ready.unwrap_or(false))
}

/// The screen's centre (`Settings.getVideoMode()`).
fn screen_centre(engine: &GameEngine) -> Result<(i64, i64), String> {
	let w = eval_i64(engine, "::Settings.getVideoMode().Width / 2")?.ok_or("no video mode")?;
	let h = eval_i64(engine, "::Settings.getVideoMode().Height / 2")?.ok_or("no video mode")?;
	Ok((w, h))
}

/// Click the tile at square coords `(x, y)`: centre the camera on it, then move the
/// mouse there, press and release the left button.
pub fn click_tile(engine: &GameEngine, x: i64, y: i64) -> Result<(), String> {
	engine.run_script(&format!("::Tactical.getCamera().setPos(::Tactical.getTileSquare({x}, {y}).Pos);"), "player")?;
	let (cx, cy) = screen_centre(engine)?;
	let root = engine.root_state();
	for input in [MouseInput::moved(cx, cy), MouseInput::left_down(cx, cy), MouseInput::left_up(cx, cy)] {
		root.mouse_input(engine.vm(), input)?;
	}
	Ok(())
}

/// Move the acting unit to square `(x, y)` as a player does: one click shows the path,
/// a second on the same tile goes. Whether it set off.
pub fn move_to(engine: &GameEngine, x: i64, y: i64) -> Result<bool, String> {
	click_tile(engine, x, y)?;
	let shown = eval_bool(engine, "::Tactical.State.m.CurrentActionState == ::Const.Tactical.ActionState.ComputePath")?;
	if shown != Some(true) {
		return Ok(false);
	}
	click_tile(engine, x, y)?;
	Ok(eval_bool(engine, "::Tactical.State.m.CurrentActionState == ::Const.Tactical.ActionState.TravelPath")? == Some(true))
}

/// The acting unit's `getID()`.
fn active_id(engine: &GameEngine) -> Result<i64, String> {
	eval_i64(engine, "::Tactical.TurnSequenceBar.getActiveEntity().getID()")?.ok_or_else(|| "no acting unit".to_string())
}

/// Use skill `skill_id` (e.g. `"actives.thrust"`), clicking it on the skill bar, then on
/// the target tile if it needs one.
pub fn use_skill(engine: &GameEngine, skill_id: &str, target: Option<(i64, i64)>) -> Result<(), String> {
	let id = active_id(engine)?;
	engine.run_script(
		&format!("::Tactical.State.m.TacticalScreen.m.TurnSequenceBar.onEntitySkillClicked({{ entityId = {id}, skillId = \"{skill_id}\" }});"),
		"player",
	)?;
	if let Some((x, y)) = target {
		click_tile(engine, x, y)?;
	}
	Ok(())
}

/// Press the turn bar's Wait button.
pub fn wait_turn(engine: &GameEngine) -> Result<(), String> {
	engine.run_script("::Tactical.State.m.TacticalScreen.m.TurnSequenceBar.onWaitTurnButtonPressed();", "player")
}

/// Press the turn bar's End Turn button.
pub fn end_turn(engine: &GameEngine) -> Result<(), String> {
	engine.run_script("::Tactical.State.m.TacticalScreen.m.TurnSequenceBar.onNextTurnButtonPressed();", "player")
}

/// The 'C' key: opens the inventory (the character screen), and closes it again.
pub const KEY_C: i64 = 13;

/// Press and release key `key`.
pub fn press_key(engine: &GameEngine, key: i64) -> Result<(), String> {
	let root = engine.root_state();
	root.key_input(engine.vm(), KeyInput::pressed(key))?;
	root.key_input(engine.vm(), KeyInput::released(key))?;
	Ok(())
}

/// Whether the inventory is open (shown, not still animating).
pub fn inventory_open(engine: &GameEngine) -> Result<bool, String> {
	let open = eval_bool(
		engine,
		"::Tactical.State.m.CharacterScreen.isVisible() && !::Tactical.State.m.CharacterScreen.isAnimating()",
	)?;
	Ok(open == Some(true))
}

/// Equip the acting unit's item in bag slot `slot`, as the inventory does when the
/// player right-clicks it (`onEquipBagItem`). The inventory must be open. `Ok(false)`
/// if the game refused (e.g. not enough action points); the reason is logged.
pub fn equip_bag_item(engine: &GameEngine, slot: i64) -> Result<bool, String> {
	let done = eval_bool(
		engine,
		&format!(
			"(function() {{
				local e = ::Tactical.TurnSequenceBar.getActiveEntity();
				local item = e.getItems().getItemAtBagSlot({slot});
				if (item == null) return false;
				local r = ::Tactical.State.m.CharacterScreen.onEquipBagItem([e.getID(), item.getInstanceID(), null]);
				if (typeof r == \"table\" && \"error\" in r) {{ ::logInfo(\"equip_bag_item: \" + r.error); return false; }}
				return r != null && r != false;
			}})()"
		),
	)?;
	Ok(done == Some(true))
}

/// Retreat: the top bar's Flee button, then Retreat in the dialog it opens.
pub fn retreat(engine: &GameEngine) -> Result<(), String> {
	engine.run_script("::Tactical.State.topbar_options_onFleeButtonClicked();", "player")?;
	engine.run_script("::Tactical.State.tactical_flee_screen_onFleePressed();", "player")
}

/// An integer from the acting unit (`expr` uses `e`), e.g. `"e.getActionPoints()"`.
pub fn acting_i64(engine: &GameEngine, expr: &str) -> Result<Option<i64>, String> {
	eval_i64(engine, &format!("(function(e) {{ return {expr}; }})(::Tactical.TurnSequenceBar.getActiveEntity())"))
}
