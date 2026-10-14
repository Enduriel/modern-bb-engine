// UI parity goldens of the world (see world_golden.js): what the page asks the scripts
// to do between snapshots, as a method on the world screen's script object (the page
// reaches it with SQ.call). Our side (`host::goldens::capture_world`) runs this same
// file and calls the method directly.

// This runs right after world_state.init(), before the world's first update (the probe
// harness's campaign mode runs it there; our side, `host::goldens::capture_world`, too).

// Entering a town (and starting a battle) autosaves, which in the original would
// overwrite the player's own "autosave" slot.
::World.State.m.IsAutosaving = false;

// startNewCampaign() ends with `Math.seedRandom(Time.getRealTime())`, and what the world
// rolls after it (contracts and the town situations they bring, shop stock) differs per
// run. Pin it, so both engines roll the same.
::Math.seedRandom(20261013);

::World.State.m.WorldScreen.probeStep <- function ( _step )
{
	local state = ::World.State;

	if (_step == "character")
	{
		// The top bar's Brothers button.
		state.topbar_options_module_onBrothersButtonClicked();
	}
	else if (_step == "closeCharacter")
	{
		state.character_screen_onClosePressed();
	}
	else if (_step == "town")
	{
		// The enterable settlement nearest the company (the first of equals).
		local tile = state.getPlayer().getTile();
		local nearest = null;
		local distance = 0;

		foreach( s in ::World.EntityManager.getSettlements() )
		{
			if (!s.isEnterable())
			{
				continue;
			}

			local d = s.getTile().getDistanceTo(tile);

			if (nearest == null || d < distance)
			{
				nearest = s;
				distance = d;
			}
		}

		::logInfo("PROBE town " + nearest.getName() + " at " + distance);
		state.enterLocation(nearest);
	}
	else if (_step == "leaveTown")
	{
		state.town_screen_main_dialog_module_onLeaveButtonClicked();
	}
	else if (_step == "combat")
	{
		// A bandit ambush as vanilla's events start one, with what would vary between
		// runs pinned, as the draw-exact battle traces pin it
		// (bb-engine/tests/combat_trace.rs): idle sounds, which fire on frame timing;
		// the wall-clock reseed at the end of the spawn; the combat seed (from the C
		// runtime's rand()); and the generator, which the steps before drew from.
		::Const.Sound.IdleSoundBaseDelay = 1.0e9;
		::Const.Sound.IdleSoundMinDelay = 1.0e9;
		local seedRandom = ::Math.seedRandom;
		::Math.seedRandom = function ( _s )
		{
			return seedRandom(_s == ::Time.getRealTime() ? 999 : _s);
		};
		state.m.CombatSeed = 777;
		::Math.seedRandom(4242);
		local properties = state.getLocalCombatProperties(state.getPlayer().getPos());
		properties.CombatID = "Event";
		properties.Music = ::Const.Music.BanditTracks;
		properties.IsAutoAssigningBases = false;
		properties.Entities = [];
		::Const.World.Common.addUnitsToCombat(properties.Entities, ::Const.World.Spawn.BanditDefenders, 100, ::Const.Faction.Enemy);
		state.startScriptedCombat(properties, false, false, true);
	}
	else if (_step == "playerTurn")
	{
		// Whether the battle waits for the player: one of theirs is up.
		try
		{
			local active = ::Tactical.TurnSequenceBar.getActiveEntity();
			return active != null && active.isPlayerControlled();
		}
		catch( e )
		{
			return false;
		}
	}

	return null;
};
::logInfo("PROBE sq: world golden steps ready");
