// Frame parity scenes: the game put into fixed states that both engines then draw. The
// original's page (scenes.js) and ours (bb-app's frame_scenes) walk ::probeSteps,
// calling probeScene for each. Runs right after world_state.init(), before the world's
// first update (the probe harness's campaign mode; ours does the same).

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

// The steps, in order. Each: `scene`, the probeScene to call (null for none); `shot`,
// the screenshot's name (null for none); `until`, a probeScene to poll until it returns
// true before shooting; `click`, a button to click on the page afterwards.
::probeSteps <- [
	{ scene = null, shot = "intro", until = null, click = "Huzzah!" },
	{ scene = "world", shot = "world", until = null, click = null },
	{ scene = "town", shot = "town", until = null, click = null },
	{ scene = "leaveTown", shot = null, until = null, click = null },
	{ scene = "night", shot = "night", until = "nightFell", click = null },
	{ scene = "battle", shot = "battle_turn", until = "playerTurn", click = null },
	{ scene = "pathShort", shot = "battle_path_short", until = null, click = null },
	{ scene = "pathLong", shot = "battle_path_long", until = null, click = null },
	{ scene = "skill", shot = "battle_skill", until = null, click = null },
	{ scene = "battleEffects", shot = "battle_effects", until = null, click = null },
	{ scene = "winBattle", shot = "battle_results", until = "resultsShown", click = "Continue" },
	{ scene = null, shot = "battle_loot", until = null, click = null }
];

// The empty tile `_steps` tiles from `_from` in direction `_dir`, if there is one.
local function tileAway( _from, _dir, _steps )
{
	local tile = _from;

	for( local i = 0; i < _steps; i++ )
	{
		if (!tile.hasNextTile(_dir))
		{
			return null;
		}

		tile = tile.getNextTile(_dir);
	}

	return tile.IsEmpty ? tile : null;
}

// What hovering `_tile` shows the active unit: its path and what it costs, as
// tactical_state.computeEntityPath does for the tile under the mouse.
local function showPath( _tile )
{
	local state = ::Tactical.State;
	local e = ::Tactical.TurnSequenceBar.getActiveEntity();
	state.m.LastTileSelected = _tile;
	state.m.CurrentActionState = ::Const.Tactical.ActionState.ComputePath;
	local settings = ::Tactical.getNavigator().createSettings();
	settings.ActionPointCosts = e.getActionPointCosts();
	settings.FatigueCosts = e.getFatigueCosts();
	settings.FatigueCostFactor = ::Const.Movement.FatigueCostFactor;
	settings.ActionPointCostPerLevel = e.getLevelActionPointCost();
	settings.FatigueCostPerLevel = e.getLevelFatigueCost();
	settings.ZoneOfControlCost = 4;
	settings.AlliedFactions = e.getAlliedFactions();
	settings.Faction = e.getFaction();
	settings.AllowZoneOfControlPassing = true;
	settings.IsPlayer = true;

	if (!::Tactical.getNavigator().findPath(e.getTile(), _tile, settings, 0))
	{
		return null;
	}

	::Tactical.getNavigator().buildVisualisation(e, settings, e.getActionPoints(), e.getFatigueMax() - e.getFatigue());
	::Tactical.getHighlighter().clear();
	::Tactical.getHighlighter().highlightZoneOfControl(e.getAlliedFactions());
	settings.ZoneOfControlCost = 0;
	local costs = ::Tactical.getNavigator().getCostForPath(e, settings, e.getActionPoints(), e.getFatigueMax() - e.getFatigue());

	if (costs.Tiles != 0)
	{
		::Tactical.TurnSequenceBar.setActiveEntityCostsPreview(costs);
	}

	return "path of " + costs.Tiles + " tiles, " + costs.ActionPointsRequired + " AP";
}

::World.State.m.WorldScreen.probeScene <- function ( _scene )
{
	local state = ::World.State;
	local player = state.getPlayer();
	local note = "";
	// No mouse moves over the probe's game: put the scripts' cursor over the UI's top-left
	// panel in both engines (hovering hits nothing on the map there, and the tooltips
	// don't divide by a cursor that was never set), drawn by the system (so not in
	// either frame).
	::UI.setCursorHardware(true);
	::Cursor.setPosition(40, 20);

	if (_scene == "steps")
	{
		return ::probeSteps;
	}
	else if (_scene == "world")
	{
		// The company's spot on the map, the camera on it at the default zoom, paused.
		state.setPause(true);
		::World.getCamera().Zoom = 1.0;
		::World.getCamera().setPos(player.getPos());
	}
	else if (_scene == "town")
	{
		// The enterable settlement nearest the company (the first of equals).
		local tile = player.getTile();
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

		note = nearest.getName();
		// Its shop and recruits come from seeds that hold the real time at the town's
		// creation (settlement.create): pin them, as parity/probes/campaign/economy.nut
		// does, so both engines show the same.
		nearest.m.ShopSeed = 1000;
		nearest.m.RosterSeed = 2000;
		state.enterLocation(nearest);
	}
	else if (_scene == "leaveTown")
	{
		state.town_screen_main_dialog_module_onLeaveButtonClicked();
	}
	else if (_scene == "night" || _scene == "nightFell" && ::World.getTime().IsDaytime)
	{
		// Time runs fast until night falls ("nightFell"), again whenever something (the
		// world's auto-pause after leaving a town) stops it.
		if (::World.getSpeedMult() != 8.0)
		{
			state.setAutoPause(false);
			state.setPause(false);
			::World.setSpeedMult(8.0);
		}

		if (_scene == "nightFell")
		{
			return false;
		}
	}
	else if (_scene == "nightFell")
	{
		state.setPause(true);
		::World.setSpeedMult(1.0);
		::World.getCamera().Zoom = 1.0;
		::World.getCamera().setPos(player.getPos());
		note = "day " + ::World.getTime().Days + " time of day " + ::World.getTime().TimeOfDay;
	}
	else if (_scene == "battle")
	{
		// A bandit ambush as vanilla's events start one, with what would vary between
		// runs pinned (bb-ui/probes/world_golden.nut's "combat"; the draw-exact
		// battle tests pin it the same way).
		::Const.Sound.IdleSoundBaseDelay = 1.0e9;
		::Const.Sound.IdleSoundMinDelay = 1.0e9;
		local seedRandom = ::Math.seedRandom;
		::Math.seedRandom = function ( _s )
		{
			return seedRandom(_s == ::Time.getRealTime() ? 999 : _s);
		};
		state.m.CombatSeed = 777;
		// Paused first: until the loading screen shows, the world's background work runs
		// on, and a roaming party's path search draws numbers each frame it does, which
		// would move the battle map by however many frames that took.
		state.setPause(true);
		::Math.seedRandom(4242);
		local properties = state.getLocalCombatProperties(player.getPos());
		properties.CombatID = "Event";
		properties.Music = ::Const.Music.BanditTracks;
		properties.IsAutoAssigningBases = false;
		properties.Entities = [];
		::Const.World.Common.addUnitsToCombat(properties.Entities, ::Const.World.Spawn.BanditDefenders, 100, ::Const.Faction.Enemy);
		state.startScriptedCombat(properties, false, false, true);
	}
	else if (_scene == "playerTurn")
	{
		// Whether the battle waits for the player: one of theirs is up.
		try
		{
			local active = ::Tactical.TurnSequenceBar.getActiveEntity();
			return active != null && active.isPlayerControlled() && !::Tactical.State.isInputLocked();
		}
		catch( e )
		{
			return false;
		}
	}
	else if (_scene == "pathShort" || _scene == "pathLong")
	{
		// The first tile, by distance then direction, the active unit has a path to: 2
		// tiles off, or 9 (beyond its action points), else as far as there is one.
		local e = ::Tactical.TurnSequenceBar.getActiveEntity();
		local distances = _scene == "pathShort" ? [2, 3] : [9, 8, 7, 10, 11, 6];
		note = "no path";

		foreach( d in distances )
		{
			for( local dir = 0; dir < 6 && note == "no path"; dir++ )
			{
				local tile = tileAway(e.getTile(), dir, d);
				local shown = tile == null ? null : showPath(tile);

				if (shown != null)
				{
					note = shown + " (" + d + " tiles off, direction " + dir + ")";
				}
			}

			if (note != "no path")
			{
				break;
			}
		}
	}
	else if (_scene == "skill")
	{
		::Tactical.getNavigator().clearVisualisation();
		::Tactical.getHighlighter().clear();
		::Tactical.TurnSequenceBar.resetActiveEntityCostsPreview();
		::Tactical.State.setActionStateBySkillIndex(0);
		note = "" + ::Tactical.State.m.SelectedSkillID;
	}

	else if (_scene == "battleEffects")
	{
		// Battle effects, all in one shot. Virtual time is frozen first,
		// so both engines shoot each effect as spawned (the original's frames are real
		// time, so a moment mid-flight couldn't be matched). Each effect is on a unit of
		// its own, or on open ground between the two sides, where the camera looks.
		// Nothing here draws from the engine's own random stream (no varyColor), which the
		// two engines don't share. Needs three brothers and three enemies.
		local byId = function ( _a, _b )
		{
			return _a.getID() <=> _b.getID();
		};
		local bros = ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player).slice(0);
		local foes = [];

		foreach( faction in ::Tactical.Entities.getAllInstances() )
		{
			foreach( e in faction )
			{
				if (e.getFaction() != ::Const.Faction.Player && e.isAlive())
				{
					foes.push(e);
				}
			}
		}

		bros.sort(byId);
		foes.sort(byId);

		// The skill the last step picked, put down.
		::Tactical.getNavigator().clearVisualisation();
		::Tactical.State.cancelEntitySkill(::Tactical.TurnSequenceBar.getActiveEntity());
		::Time.setVirtualSpeed(0.0);

		// The ground halfway between the second brother and the first enemy.
		local a = bros[1].getTile().SquareCoords;
		local b = foes[0].getTile().SquareCoords;
		local mid = ::Tactical.getTileSquare((a.X + b.X) / 2, (a.Y + b.Y) / 2);
		local s = ::Const.Tactical.Settings;
		// A skill's icon over the first brother.
		::Tactical.spawnIconEffect("status_effect_00", bros[0].getTile(), s.SkillIconOffsetX, s.SkillIconOffsetY, s.SkillIconScale, s.SkillIconFadeInDuration, s.SkillIconStayDuration, s.SkillIconFadeOutDuration, s.SkillIconMovement);
		// An arrow leaving the middle ground for the first enemy.
		::Tactical.spawnProjectileEffect(::Const.ProjectileSprite[::Const.ProjectileType.Arrow], mid, foes[0].getTile(), 1.0, 1.0, true, false);
		// A blood pool beside it (the first decal, at its terrain's alpha).
		local pool = ::Tactical.getTileSquare(mid.SquareCoords.X, mid.SquareCoords.Y + 2);
		::Tactical.spawnPoolEffect(::Const.BloodPoolDecals[::Const.BloodType.Red][0], pool, ::Const.BloodPoolTerrainAlpha[pool.Type], 1.0, ::Const.Tactical.DetailFlag.Corpse);
		// The third brother's head darker, the second enemy's tinted red.
		bros[2].getSprite("head").setBrightness(0.5);
		foes[1].getSprite("head").Color = ::createColor("#ff6060");
		// The third enemy beheaded by the first brother: the head as it leaves, the
		// corpse, and the blood the scripts spawn.
		foes[2].kill(bros[0], null, ::Const.FatalityType.Decapitated);

		local camera = ::Tactical.getCamera();
		camera.setPos(mid.Pos);
		camera.Level = camera.getBestLevelForTile(mid);
		note = "middle " + mid.SquareCoords.X + "," + mid.SquareCoords.Y + "; pool " + pool.SquareCoords.X + "," + pool.SquareCoords.Y;
	}
	else if (_scene == "winBattle")
	{
		// The battle won at once: time runs again, and every enemy left dies.
		::Time.setVirtualSpeed(1.0);

		foreach( faction in ::Tactical.Entities.getAllInstances() )
		{
			foreach( e in faction.slice(0) )
			{
				if (e.getFaction() != ::Const.Faction.Player && e.isAlive())
				{
					e.kill();
				}
			}
		}
	}
	else if (_scene == "resultsShown")
	{
		local results = ::Tactical.State.m.TacticalCombatResultScreen;
		return results != null && results.isVisible() && !results.isAnimating();
	}

	if (_scene != "nightFell" && _scene != "playerTurn" && _scene != "resultsShown" || note != "")
	{
		::logInfo("PROBE scene " + _scene + " = " + note);
	}
	return true;
};
::logInfo("PROBE sq: frame scenes ready");
