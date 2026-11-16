// mode: campaign seed=rootcalls
// Which root-table functions the natives call through a script's replacement: each one is
// wrapped, and a call made by a native is recorded. Around a native the probe calls, every
// call of a wrapper counts; while the game runs (on the map, then in a battle with the AI
// paused) a call whose caller is a native frame or no frame at all does. A group of its own:
// it fights, and waits (`BbProbe.DeferDone`).

::BbProbe.DeferDone = true;
::__rootCalls <- {
	Window = null,
	Hits = {},
	Phase = null,
	Waited = 0,
	Wrapped = [
		"Time.getDelta", "Time.getVirtualSpeed", "Time.getRealTime", "Time.getRealTimeF", "Time.getVirtualTimeF",
		"World.getMapSize", "World.isVisible", "World.isUsingFogOfWar", "World.getSpeedMult", "World.worldToTile",
		"World.getTile", "World.uncoverFogOfWar", "Tactical.getMapSize", "Settings.getGameplaySettings",
		"Settings.getTempGameplaySettings", "Settings.getControlSettings", "Settings.getVideoMode", "createVec",
		"createColor", "logError", "isKindOf"
	],
	function log( _k, _v ) { ::logInfo("PROBE " + _k + " = " + typeof _v + " " + (_v == null ? "null" : _v)); },
	function later( _ms, _f ) { ::Time.scheduleEvent(::TimeUnit.Real, _ms, _f, null); },
	function hits()
	{
		local keys = [];
		foreach (k, _ in ::__rootCalls.Hits) keys.push(k);
		keys.sort();
		::__rootCalls.Hits = {};
		return keys.len() == 0 ? "none" : keys.reduce(@(a, b) a + ", " + b);
	},
	function wrap( _path )
	{
		local parts = split(_path, "."), owner = parts.len() == 2 ? ::getroottable()[parts[0]] : ::getroottable();
		local name = parts.top(), orig = owner[name];
		owner[name] = function( ... )
		{
			local R = ::__rootCalls;
			if (R.Window != null) R.Hits[_path] <- true;
			else if (R.Phase != null)
			{
				local who = ::getstackinfos(2);
				if (who == null) R.Hits[_path + " from nothing"] <- true;
				else if (who.src == "NATIVE") R.Hits[_path + " from native " + who.func] <- true;
			}
			local args = [this];
			args.extend(vargv);
			return orig.acall(args);
		};
	},
	// What the probe's own native call made of the wrappers.
	function around( _k, _f )
	{
		local R = ::__rootCalls, v;
		R.Hits = {};
		R.Window = _k;
		try { _f(); v = "ok"; } catch (e) { v = "ERROR " + e; }
		R.Window = null;
		R.log(_k, v + ", calls " + R.hits());
	},
	function controls()
	{
		local R = ::__rootCalls, sortFrame = null;
		[2, 1].sort(function( _a, _b )
		{
			local w = ::getstackinfos(2);
			sortFrame = w == null ? "nothing" : w.src + " " + w.func;
			return _a <=> _b;
		});
		R.log("control: sort's comparator called from", sortFrame);
	},
	function onMap( _ )
	{
		local R = ::__rootCalls;
		local screen = ::World.State.getEventScreen();
		if (screen != null && screen.isVisible()) screen.onButtonPressed(0);
		R.around("World.update", @() ::World.update());
		local weather = ::World.getWeather(), clouds = weather.createCloudSettings(), birds = weather.createBirdSettings();
		R.around("World weather createCloudSettings", @() weather.createCloudSettings());
		R.around("World weather createRainSettings", @() weather.createRainSettings());
		R.around("World weather buildCloudCover", @() weather.buildCloudCover(clouds));
		R.around("World weather buildBirds", @() weather.buildBirds(birds));
		R.around("World.screenToTile", @() ::World.screenToTile(100, 100));
		R.around("World camera getPos", @() ::World.getCamera().getPos());
		R.around("World camera screenToWorld", @() ::World.getCamera().screenToWorld(100, 100));
		R.around("execute of a syntax error", @() ::execute("local x = ("));
		local t1 = ::World.getTileSquare(10, 10), t2 = ::World.getTileSquare(11, 10), player = ::World.State.getPlayer();
		local a = ::createVec(0, 0), b = ::createVec(10, 10), at = player.getPos();
		R.around("World tile Pos", @() t1.Pos);
		R.around("World tile getPosBetweenThisAnd", @() t1.getPosBetweenThisAnd(t2));
		R.around("World.getPlayerPos", @() ::World.getPlayerPos());
		R.around("World.move", @() ::World.move(a, b, 1.0));
		R.around("party getSpriteOffset", @() player.getSpriteOffset("body"));
		R.around("World.getAllEntitiesAndOneLocationAtPos", @() ::World.getAllEntitiesAndOneLocationAtPos(at, 100.0));
		R.Phase = "map";
		::World.State.setPause(false);
		R.later(3000, R.start);
	},
	function start( _ )
	{
		local R = ::__rootCalls;
		R.Phase = null;
		R.log("map frames", R.hits());
		local st = ::World.State;
		st.setPause(true);
		local p = st.getLocalCombatProperties(st.getPlayer().getPos());
		p.CombatID = "Event";
		p.IsAutoAssigningBases = false;
		p.Entities = [];
		::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditDefenders, 100, ::Const.Faction.Enemy);
		st.startScriptedCombat(p, false, false, true);
		R.later(50, R.wait);
	},
	function wait( _ )
	{
		local R = ::__rootCalls;
		local up = ("State" in ::Tactical) && ::Tactical.State != null;
		if (up) ::Tactical.State.m.IsAIPaused = true;
		up = up && ::Tactical.TurnSequenceBar.getActiveEntity() != null;
		if (!up && ++R.Waited < 1200)
		{
			R.later(50, R.wait);
			return;
		}
		if (!up)
		{
			R.log("battle", "ERROR the battle never began");
			::BbProbe.done();
			return;
		}
		R.Hits = {};
		R.Phase = "battle";
		R.later(3000, R.inBattle);
	},
	function inBattle( _ )
	{
		local R = ::__rootCalls;
		R.Phase = null;
		R.log("battle frames", R.hits());
		local e = ::Tactical.TurnSequenceBar.getActiveEntity(), v0 = ::createVec(0, 0), v1 = ::createVec(0, 10);
		R.around("Tactical.update", @() ::Tactical.update());
		R.around("Tactical.tileToWorld", @() ::Tactical.tileToWorld(3, 3));
		R.around("unit getPos", @() e.getPos());
		R.around("unit getRelativePos", @() e.getRelativePos());
		R.around("unit getSpriteOffset", @() e.getSpriteOffset("body"));
		R.around("unit moveSpriteOffset", @() e.moveSpriteOffset("body", v0, v1, 100, 0));
		local weather = ::Tactical.getWeather(), clouds = weather.createCloudSettings();
		R.around("Tactical weather buildCloudCover", @() weather.buildCloudCover(clouds));
		R.around("Tactical weather createRainSettings", @() weather.createRainSettings());
		R.later(1000, R.hop);
	},
	// An enemy's hop to an empty tile two away from every unit, and the frames of the next 3 s.
	function hop( _ )
	{
		local R = ::__rootCalls, size = ::Tactical.getMapSize(), dest = null, enemy = null;
		foreach (u in ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Enemy))
			if (u.isPlacedOnMap() && u.isAlive() && u.getID() != ::Tactical.TurnSequenceBar.getActiveEntity().getID()) enemy = u;
		for (local x = 3; x < size.X - 3 && dest == null; x++)
			for (local y = 3; y < size.Y - 3 && dest == null; y++)
			{
				local t = ::Tactical.getTileSquare(x, y);
				if (!t.IsEmpty || !t.IsVisibleForPlayer || t.getDistanceTo(enemy.getTile()) != 2) continue;
				local ok = true;
				foreach (a in ::Tactical.Entities.getAllInstancesAsArray())
					if (a.isPlacedOnMap() && a.getTile().getDistanceTo(t) < 2) ok = false;
				if (ok) dest = t;
			}
		local nav = ::Tactical.getNavigator();
		R.around("navigator teleport", @() nav.teleport(enemy, dest, null, null, false, 1.0));
		R.Phase = "hop";
		R.later(3000, R.landed);
	},
	function landed( _ )
	{
		local R = ::__rootCalls;
		R.Phase = null;
		R.log("hop frames", R.hits());
		::BbProbe.done();
	}
};
::__rootCalls.controls();
foreach (path in ::__rootCalls.Wrapped) ::__rootCalls.wrap(path);
::__rootCalls.later(2500, ::__rootCalls.onMap);
