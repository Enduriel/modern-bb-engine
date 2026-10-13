// UI parity goldens of the world (see world_golden.js): what the page asks the scripts
// to do between snapshots, as a method on the world screen's script object (the page
// reaches it with SQ.call). Our side (`host::goldens::capture_world`) runs this same
// file and calls the method directly.

// Entering a town (and starting a battle) autosaves, which in the original would
// overwrite the player's own "autosave" slot.
::World.State.m.IsAutosaving = false;

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

	return null;
};
::logInfo("PROBE sq: world golden steps ready");
