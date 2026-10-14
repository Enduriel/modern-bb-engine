// The battle both engines start for the tactical screenshots: a bandit fight on
// tactical.hills, set up as bb-engine/tests/combat_scenarios.rs does (draw-exact
// against the original), with the generators pinned at the call that starts it.
::World.State.m.IsAutosaving = false;
::Const.Sound.IdleSoundBaseDelay = 1.0e9;
::Const.Sound.IdleSoundMinDelay = 1.0e9;
::__battleSeedRandom <- ::Math.seedRandom;
::Math.seedRandom = function ( _s )
{
	return ::__battleSeedRandom(_s == ::Time.getRealTime() ? 999 : _s);
};
::__startBattle <- function()
{
	local st = ::World.State.get();
	local orig = st.startScriptedCombat;
	st.startScriptedCombat <- function( ... )
	{
		if (vargv.len() == 0)
		{
			::Math.seedRandom(4242);
			this.m.CombatSeed = 777;
		}
		local args = [this];
		args.extend(vargv);
		return orig.acall(args);
	};
	::Math.seedRandom(4242);
	local p = st.getLocalCombatProperties(st.getPlayer().getPos(), true);
	p.Entities = [];
	p.Parties = [];
	p.TerrainTemplate = "tactical.hills";
	p.CombatID = "Trace";
	::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditDefenders, 100, ::World.FactionManager.getFactionOfType(::Const.FactionType.Bandits).getID());
	st.startScriptedCombat(p, true, true, true);
};
// What a player does until the battle: the open event's first button, then engage once.
::__engaged <- false;
::__pilot <- function()
{
	if (("State" in ::Tactical) && ::Tactical.State != null) return;
	local state = ::World.State;
	local screen = state.getEventScreen();
	if (screen != null && screen.isVisible() && !screen.isAnimating()) screen.onButtonPressed(0);
	local dialog = state.m.CombatDialog;
	if (dialog != null && dialog.isVisible() && !dialog.isAnimating() && !::__engaged)
	{
		::__engaged = true;
		dialog.onEngageButtonPressed();
	}
};
::__playerTurn <- function()
{
	try
	{
		local active = ::Tactical.TurnSequenceBar.getActiveEntity();
		return active != null && active.isPlayerControlled();
	}
	catch( e )
	{
		return false;
	}
};
// The screenshot's camera: zoom 1 on the centre tile.
::__battleCamera <- function()
{
	local size = ::Tactical.getMapSize();
	local tile = ::Tactical.getTileSquare(size.X / 2, size.Y / 2);
	::Tactical.getCamera().Zoom = 1.0;
	::Tactical.getCamera().setPos(tile.Pos);
	return tile.SquareCoords.X + "," + tile.SquareCoords.Y + " pos " + tile.Pos.X + "," + tile.Pos.Y + " level " + tile.Level;
};
::__fogOff <- function()
{
	::Tactical.State.m.IsFogOfWarVisible = false;
	::Tactical.fillVisibility(::Const.Faction.Player, true);
};
