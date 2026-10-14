// Frame parity scenes (the original's side): the game put into a fixed state the page
// then screenshots. Runs right after world_state.init(), before the world's first update.

// Entering a town or a battle autosaves, which would overwrite the player's own slot.
::World.State.m.IsAutosaving = false;

// What the world rolls after worldgen depends on the wall clock: pin it, as the UI
// goldens do (bb-ui/probes/world_golden.nut).
::Math.seedRandom(20261013);

// The screen both engines draw: the original keeps its configured video mode (a mode set
// at runtime doesn't take under the probe's gamescope), so ours draws at the one logged
// here (`frame_scenes --screen`).
local mode = ::Settings.getVideoMode();
::logInfo("PROBE video mode = " + mode.Width + "x" + mode.Height + " ui " + mode.UIScale + " scene " + mode.SceneScale
	+ " depth of field " + mode.DepthOfField + " fullscreen " + mode.Fullscreen + " borderless " + mode.Borderless);

::World.State.m.WorldScreen.probeScene <- function ( _scene )
{
	local state = ::World.State;
	local player = state.getPlayer();

	if (_scene == "world")
	{
		// The company's spot on the map, the camera on it at the default zoom, paused.
		state.setPause(true);
		::World.getCamera().Zoom = 1.0;
		::World.getCamera().setPos(player.getPos());
	}

	try
	{
		local m = ::Settings.getVideoMode();
		::logInfo("PROBE scene " + _scene + " = zoom " + ::World.getCamera().Zoom
			+ " player " + player.getPos().X + "," + player.getPos().Y
			+ " screen " + m.Width + "x" + m.Height + " ui " + m.UIScale + " scene " + m.SceneScale);
	}
	catch (e)
	{
		::logInfo("PROBE scene " + _scene + " = (state not logged: " + e + ")");
	}
	return true;
};
::logInfo("PROBE sq: frame scenes ready");
