// Performance baselines in the original, a battle's start (campaign mode, seed rust-engine-test): perf.py
// prepends original_common.nut. Runs after world_state.init(): the intro event dismissed,
// the world run 2 s, then brigands from vanilla's spawn list next to the company, the
// combat dialog, Engage once it has finished animating, timed to the battle running
// behind no loading screen (as bb-app's load_times example times ours). Autosaving off,
// so the player's autosave isn't touched; nothing is saved.

local function p( _k, _v )
{
	::logInfo("PROBE perf " + _k + " = " + _v);
}

::BbProbe.DeferDone = true;
::World.State.m.IsAutosaving = false;

::__perfBattle <- function ()
{
	local player = ::World.State.getPlayer();
	::World.FactionManager.getFactionOfType(::Const.FactionType.Bandits).spawnEntity(player.getTile(), "Brigand Raiders", false, ::Const.World.Spawn.BanditRaiders, 100);
	p("mark battle", "t=" + ::Time.getExactTime());
	::__perfBattleStart <- ::Time.getExactTime();
	::__perfBattleEngaged <- false;
	::__perfBattleScreenUp <- null;
	::World.State.showCombatDialog(true, true, true, null, player.getPos());
	::__perfBattleWait(2400);
};

::__perfBattleWait <- function ( _tries )
{
	local since = ::Time.getExactTime() - ::__perfBattleStart;
	local dialog = ::World.State.m.CombatDialog;
	if (!::__perfBattleEngaged && dialog != null && dialog.isVisible() && !dialog.isAnimating())
	{
		::__perfBattleEngaged = true;
		dialog.onEngageButtonPressed();
	}
	if (::__perfBattleScreenUp == null && ::LoadingScreen.isVisible() && !::LoadingScreen.isAnimating())
	{
		::__perfBattleScreenUp = since;
		p("loading_screen_up_s", since);
	}
	if (("State" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible())
	{
		p("battle_start_s", since);
		p("mark end", "t=" + ::Time.getExactTime());
		::BbProbe.done();
	}
	else if (_tries > 0)
	{
		::Time.scheduleEvent(::TimeUnit.Real, 16, function ( _d ) { ::__perfBattleWait(_tries - 1); }, null);
	}
	else
	{
		p("battle_start_s", "never");
		::BbProbe.done();
	}
};

::__perfWaitIntro <- function ( _tries )
{
	if (::World.State.m.EventScreen.isVisible())
	{
		::World.State.m.EventScreen.onButtonPressed(0);
		::World.State.setPause(false);
		::Time.scheduleEvent(::TimeUnit.Real, 2000, function ( _d ) { ::__perfBattle(); }, null);
	}
	else if (_tries > 0)
	{
		::Time.scheduleEvent(::TimeUnit.Real, 250, function ( _d ) { ::__perfWaitIntro(_tries - 1); }, null);
	}
	else
	{
		p("battle_start_s", "no intro event");
		::BbProbe.done();
	}
};
::__perfWaitIntro(240);
