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
	{ scene = "leaveTown", shot = null, until = "leftTown", click = null },
	{ scene = "night", shot = "night", until = "nightFell", click = null },
	{ scene = "battle", shot = "battle_turn", until = "playerTurn", click = null },
	{ scene = "pathShort", shot = "battle_path_short", until = null, click = null },
	{ scene = "pathLong", shot = "battle_path_long", until = null, click = null },
	{ scene = "skill", shot = "battle_skill", until = null, click = null },
	{ scene = "battleEffects", shot = "battle_effects", until = null, click = null },
	{ scene = "rise", shot = null, until = null, click = null },
	{ scene = "rise40", shot = "battle_rise_40", until = null, click = null },
	{ scene = "rise80", shot = "battle_rise_80", until = null, click = null },
	{ scene = "occl", shot = "battle_occlusion", until = null, click = null },
	{ scene = "occlCut", shot = "battle_occlusion_cut", until = null, click = null },
	{ scene = "orient", shot = "battle_orientation", until = null, click = null },
	{ scene = "orientHidden", shot = "battle_orientation_hidden", until = null, click = null },
	{ scene = "orientZoom", shot = null, until = null, click = null },
	{ scene = "orientOff", shot = "battle_orientation_offscreen", until = null, click = null },
	{ scene = "occlClear", shot = null, until = null, click = null },
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

// The orientation overlays the battle camera reports (queryEntityOverlays), those that
// show: whose (name and tile), where, offscreen or not, and how much lower.
local function overlayNote()
{
	local shown = [];
	local all = ::Tactical.getCamera().queryEntityOverlays();
	foreach( o in all )
	{
		if (!("id" in o) || !o.visible) continue;
		local e = ::Tactical.getEntityByID(o.id);
		local t = e.getTile();
		shown.push(e.getName() + "@" + t.SquareCoords.X + "," + t.SquareCoords.Y + " " + o.x + "," + o.y
			+ (o.isOffscreen ? " offscreen" : "") + " +" + o.yOffset);
	}
	local text = "";
	foreach( i, line in shown )
		text += (i > 0 ? "; " : "") + line;
	return all.len() + " overlays, " + shown.len() + " shown: " + text;
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
	else if (_scene == "leftTown")
	{
		// Out once the town screen is gone. A Leave while its screen still animates in is
		// ignored (the page's animation runs on the clock, so a busy machine can get
		// there first): press it again once the screen has settled.
		local screen = state.m.WorldTownScreen;
		if (state.m.LastEnteredTown == null && !screen.isVisible())
		{
			return true;
		}

		if (screen.isVisible() && !screen.isAnimating())
		{
			state.town_screen_main_dialog_module_onLeaveButtonClicked();
		}

		return false;
	}
	else if (_scene == "night" || _scene == "nightFell" && ::World.getTime().IsDaytime)
	{
		// Time runs fast until night falls ("nightFell"), again whenever something (the
		// world's auto-pause after leaving a town) stops it.
		// Hours of waiting on the open map: a hostile party could catch the company
		// (barbarians did, once in a few runs: how many frames the steps before take
		// follows the page's wall-clock animations, so where the world's parties are by
		// now varies). Its combat dialog would hold the wait (a menu up, time all but
		// stopped). As vanilla's contracts keep a party out of the AI's fights, the
		// company is no one's target until night has fallen.
		player.setAttackableByAI(false);

		if (::World.getSpeedMult() != 8.0)
		{
			state.setAutoPause(false);
			state.setPause(false);
			::World.setSpeedMult(8.0);
		}

		if (_scene == "nightFell")
		{
			// Every 40th poll (20 s of game time where frame_scenes polls), how far the
			// day has come and what could hold it (runs reach night in 80-100 s of 180).
			if (!("probeNightPolls" in ::getroottable()))
			{
				::probeNightPolls <- 0;
			}

			if (++::probeNightPolls % 40 == 0)
			{
				::logInfo("night wait: day " + ::World.getTime().Days + " hour " + ::World.getTime().Hours + " speed " + ::World.getSpeedMult() +
					" paused " + state.isPaused() + " menus " + state.m.MenuStack.hasBacksteps() + " event " + ::World.Events.hasActiveEvent() +
					" at " + player.getPos().X + "," + player.getPos().Y);
			}

			return false;
		}
	}
	else if (_scene == "nightFell")
	{
		player.setAttackableByAI(true);
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
		// Where the occlusion scenes' clean-up puts the camera back.
		::probeEffectsCamera <- { tile = mid, level = camera.Level };
		note = "middle " + mid.SquareCoords.X + "," + mid.SquareCoords.Y + "; pool " + pool.SquareCoords.X + "," + pool.SquareCoords.Y;
	}
	else if (_scene == "rise")
	{
		// A zombie spawned next to the company, rising as actor.riseFromGround's rising
		// branch has it (without its particles), with virtual time frozen so each shot
		// pins how far up it is (onRender still runs every frame).
		::Time.setVirtualSpeed(0.0);
		local cam = ::Tactical.getCamera();
		local S = ::Const.Direction.S;
		local bros = ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player);
		local best = null, bd = 0;
		local size = ::Tactical.getMapSize();
		local c = bros[0].getTile().Pos;
		for (local x = 1; x < size.X - 1; x++)
			for (local y = 1; y < size.Y - 1; y++)
			{
				local t = ::Tactical.getTileSquare(x, y);
				if (!t.IsEmpty || !t.IsVisibleForPlayer || !t.hasNextTile(S)) continue;
				local s = t.getNextTile(S);
				if (t.Level != s.Level || !s.IsDiscovered || !s.IsEmpty) continue;
				local d = (t.Pos.X - c.X) * (t.Pos.X - c.X) + (t.Pos.Y - c.Y) * (t.Pos.Y - c.Y);
				if (best == null || d < bd) { best = t; bd = d; }
			}
		local e = ::Tactical.spawnEntity("scripts/entity/tactical/enemies/zombie", best.Coords.X, best.Coords.Y);
		e.setFaction(::Const.Faction.Undead);
		::probeRiser <- e;
		local p0 = e.getPos(), r0 = e.getRelativePos();
		e.setPos(::createVec(10, -30));
		local p1 = e.getPos(), r1 = e.getRelativePos();
		e.m.IsAttackable = false;
		e.m.IsRaising = true;
		e.m.RenderAnimationSpeed = 1.0;
		e.m.RenderAnimationStartTime = ::Time.getVirtualTimeF();
		e.setRenderCallbackEnabled(true);
		cam.setPos(best.Pos);
		cam.Level = cam.getBestLevelForTile(best);
		note = "tile " + best.SquareCoords.X + "," + best.SquareCoords.Y + " pos " + best.Pos.X + "," + best.Pos.Y + " level " + best.Level
			+ "; getPos " + p0.X + "," + p0.Y + " rel " + r0.X + "," + r0.Y + "; after setPos(10,-30) getPos " + p1.X + "," + p1.Y + " rel " + r1.X + "," + r1.Y
			+ "; camera level " + cam.Level;
	}
	else if (_scene.len() > 4 && _scene.slice(0, 4) == "rise")
	{
		local p = _scene.slice(4).tointeger() / 100.0;
		local e = ::probeRiser;
		e.m.RenderAnimationStartTime = ::Time.getVirtualTimeF() - p * ::Const.Combat.ResurrectAnimationTime - (p >= 1.0 ? 0.5 : 0.0);
		if (p >= 1.0 && !e.m.IsRaising) {}
		local q = e.getPos(), r = e.getRelativePos();
		note = "p " + p + " getPos " + q.X + "," + q.Y + " rel " + r.X + "," + r.Y;
	}
	else if (_scene == "occl")
	{
		// How units are hidden by what is drawn in front of them. Virtual time frozen;
		// four set-ups on tiles whose neighbours are all empty and in sight, each with the
		// levels of its tile and neighbours set by hand (Const.Direction: N 0, NE 1, SE 2,
		// S 3, SW 4, NW 5).
		::Time.setVirtualSpeed(0.0);
		local bros = ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player);
		local c = bros[0].getTile().Pos;
		local size = ::Tactical.getMapSize();
		local open = function ( _t )
		{
			return _t.IsEmpty && _t.IsDiscovered && _t.IsVisibleForPlayer;
		};
		local cands = [];
		for (local x = 2; x < size.X - 2; x++)
			for (local y = 2; y < size.Y - 2; y++)
			{
				local t = ::Tactical.getTileSquare(x, y);
				if (!open(t)) continue;
				local ok = true;
				for (local d = 0; d < 6 && ok; d++)
					ok = t.hasNextTile(d) && open(t.getNextTile(d));
				if (!ok) continue;
				local dx = t.Pos.X - c.X, dy = t.Pos.Y - c.Y;
				if (dx * dx > 900 * 900 || dy * dy > 420 * 420) continue;
				cands.push({ t = t, d = dx * dx + dy * dy, i = x * 1000 + y });
			}
		cands.sort(function ( _a, _b ) { return _a.d != _b.d ? _a.d <=> _b.d : _a.i <=> _b.i; });
		local picked = [];
		foreach( cand in cands )
		{
			local far = true;
			foreach( p in picked )
				far = far && cand.t.getDistanceTo(p) >= 4;
			if (far) picked.push(cand.t);
			if (picked.len() == 4) break;
		}
		if (picked.len() < 4)
		{
			note = "only " + picked.len() + " places";
			::logInfo("PROBE scene occl = " + note);
			return true;
		}
		// Levels: [tile, N, NE, SE, S, SW, NW].
		local setLevels = function ( _t, _l )
		{
			_t.Level = _l[0];
			local order = [0, 1, 2, 3, 4, 5];
			foreach( i, d in order )
				_t.getNextTile(d).Level = _l[i + 1];
		};
		local white = function ( _e )
		{
			foreach( name in ["bottom", "top", "body", "web"] )
				if (_e.hasSprite(name))
				{
					local s = _e.getSprite(name);
					s.Color = ::createColor("#ffffff");
					s.setHorizontalFlipping(false);
				}
		};
		local zombie = function ( _t )
		{
			local e = ::Tactical.spawnEntity("scripts/entity/tactical/enemies/zombie", _t.Coords.X, _t.Coords.Y);
			e.setFaction(::Const.Faction.Undead);
			e.m.IsAttackable = false;
			return e;
		};
		local a = picked[0], b = picked[1], cc = picked[2], d = picked[3];
		// A: at rest, the tile in front (S) a level up, the one front-left (SW) two up.
		setLevels(a, [0, 0, 0, 0, 1, 2, 0]);
		// B: sunk 100, its tile a level up, S lower, SE a level higher still.
		setLevels(b, [1, 1, 1, 2, 0, 1, 1]);
		// C: all flat; the unit pushed to the side (into SE), a tree on S.
		setLevels(cc, [0, 0, 0, 0, 0, 0, 0]);
		// D: all flat; a boulder on the tile, a unit sunk 60 behind it (N), another at rest
		// in front (S).
		setLevels(d, [0, 0, 0, 0, 0, 0, 0]);
		local za = zombie(a);
		local zb = zombie(b);
		zb.setPos(::createVec(0, -100));
		local zc = zombie(cc);
		zc.setPos(::createVec(50, -40));
		local tree = ::Tactical.spawnEntity("scripts/entity/tactical/objects/tree_lush", cc.getNextTile(3).Coords.X, cc.getNextTile(3).Coords.Y);
		white(tree);
		local boulder = ::Tactical.spawnEntity("scripts/entity/tactical/objects/boulder", d.Coords.X, d.Coords.Y);
		white(boulder);
		local zd1 = zombie(d.getNextTile(0));
		zd1.setPos(::createVec(0, -60));
		local zd2 = zombie(d.getNextTile(3));
		::probeOccl <- [za, zb, zc, tree, boulder, zd1, zd2];
		::probeOcclTiles <- [a, b, cc, d];
		local cx = (a.Pos.X + b.Pos.X + cc.Pos.X + d.Pos.X) / 4.0;
		local cy = (a.Pos.Y + b.Pos.Y + cc.Pos.Y + d.Pos.Y) / 4.0;
		local cam = ::Tactical.getCamera();
		::probeOcclCamera <- ::createVec(cx, cy);
		cam.setPos(::probeOcclCamera);
		cam.Level = 3;
		local where = function ( _t ) { return _t.SquareCoords.X + "," + _t.SquareCoords.Y + " (" + _t.Pos.X + "," + _t.Pos.Y + ")"; };
		note = "A " + where(a) + " B " + where(b) + " C " + where(cc) + " D " + where(d) + "; camera " + cx + "," + cy
			+ "; zb rel " + zb.getRelativePos().X + "," + zb.getRelativePos().Y + " zc rel " + zc.getRelativePos().X + "," + zc.getRelativePos().Y;
	}
	else if (_scene == "occlCut")
	{
		// The camera cut down to level 0: tiles above it are capped, and nothing that
		// stands on them is drawn (B's unit, on level 1).
		::Tactical.getCamera().Level = 0;
	}
	else if (_scene == "orient")
	{
		// The units' orientation overlays (the camera's round portraits): the camera
		// still at level 0, so B's unit (on level 1) shows as one over its tile, a level
		// lower; and the first brother's tile raised above the camera's level too (a
		// player's overlay is the company banner, at 3/4 size).
		local bros = ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player);
		local t = bros[0].getTile();
		t.Level = 1;
		note = "brother " + t.SquareCoords.X + "," + t.SquareCoords.Y;
	}
	else if (_scene == "orientHidden")
	{
		// The camera back over the occlusion set-ups at level 3, and A's front tile (S)
		// two levels above A: its unit is hidden behind it, the camera more than a level
		// above, so its overlay shows over A.
		note = "before: " + overlayNote();
		local a = ::probeOcclTiles[0];
		a.getNextTile(3).Level = 2;
		local cam = ::Tactical.getCamera();
		cam.setPos(::probeOcclCamera);
		cam.Level = 3;
	}
	else if (_scene == "orientZoom")
	{
		// The map fits the screen at zoom 1, so no unit can be off screen: the scene
		// scale doubled lets the camera zoom in to 0.5 (from the next frame on).
		note = "before: " + overlayNote();
		local mode = ::Settings.getVideoMode();
		::probeSceneScale <- mode.SceneScale;
		mode.SceneScale = 2.0;
		::Settings.setVideoMode(mode);
	}
	else if (_scene == "orientOff")
	{
		// Zoomed in to 0.5 over the set-ups, the camera 500 to the left of their centre:
		// the units to the right are off screen, their overlays where their line to the
		// screen's centre leaves it.
		local cam = ::Tactical.getCamera();
		cam.Zoom = 0.5;
		cam.setPos(::createVec(::probeOcclCamera.X - 500, ::probeOcclCamera.Y));
		note = "zoom " + cam.Zoom + " camera " + cam.getPos().X + "," + cam.getPos().Y;
	}
	else if (_scene == "occlClear")
	{
		note = "before: " + overlayNote();
		local mode = ::Settings.getVideoMode();
		mode.SceneScale = ::probeSceneScale;
		::Settings.setVideoMode(mode);
		::Tactical.getCamera().Zoom = 1.0;
		// What the rise and the occlusion set-ups spawned goes again (no kill: the
		// results screen and the loot stay as they were), the camera back where the
		// effects left it.
		::probeOccl.push(::probeRiser);
		foreach( e in ::probeOccl )
			if (e.isAlive()) e.die();
		::Tactical.getCamera().setPos(::probeEffectsCamera.tile.Pos);
		::Tactical.getCamera().Level = ::probeEffectsCamera.level;
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
