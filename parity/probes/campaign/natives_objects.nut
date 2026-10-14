// mode: campaign seed=nativeobjects
// compare: subset
// Coverage: the natives and properties on every other kind of native object the scripts
// get hold of, in the world (tiles, navigator, time, sprites, labels, brushes, details,
// camera, weather, the UI's handles, settings, the save stream) and then in a battle (tiles, details, units,
// sprites, camera, navigator, shaker, highlighter, weather, head effects). A group of its own: it
// changes the world, saves (a `bb_probe_` save, deleted again), and fights. It waits
// for the battle (`BbProbe.DeferDone`), so the world first runs past the scenario's
// intro event.

if (!("__nativeReport" in ::getroottable()))
{
	::__nativeGetClass <- (class {})().getclass;
	::__nativeIsNative <- function ( _v )
	{
		return (typeof _v == "function" || typeof _v == "native function") && _v.getinfos().native;
	};
	::__nativeReport <- function ( _label, _obj )
	{
		local names = {};
		local add = function ( _container )
		{
			foreach( k, v in _container )
			{
				if (typeof k == "string" && ::__nativeIsNative(v))
				{
					names[k] <- true;
				}
			}
		};

		if (typeof _obj == "table")
		{
			for( local t = _obj, depth = 0; t != null && depth < 32; t = t.getdelegate(), depth++ )
			{
				add(t);
			}
		}
		else if (typeof _obj == "instance" || typeof _obj == "class")
		{
			for( local c = typeof _obj == "instance" ? ::__nativeGetClass.call(_obj) : _obj, depth = 0; c != null && depth < 32; c = c.getbase(), depth++ )
			{
				add(c);
			}
		}

		local out = [];

		foreach( k, _ in names )
		{
			out.push(k);
		}

		out.sort();

		foreach( n in out )
		{
			::logInfo("PROBE native " + _label + "." + n + " = native");
		}
	};
}

local function report( _label, _f )
{
	try
	{
		local o = _f();
		::__nativeReport(_label, o);
		::__propReport(_label, o);
	}
	catch( e )
	{
		::logInfo("PROBE native " + _label + " = ERROR " + e);
	}
}


// Properties: the keys of the class's `__getTable` (readable) and `__setTable` (writable),
// where Sqrat keeps their accessors, up the class chain; and the read-only ones (readable,
// not writable) on lines of their own, so one writable here fails too.
::__propReport <- function ( _label, _obj )
{
	if (typeof _obj != "instance") return;
	for( local c = ::__nativeGetClass.call(_obj), depth = 0; c != null && depth < 32; c = c.getbase(), depth++ )
	{
		foreach( slot, kind in { __getTable = "get", __setTable = "set" } )
		{
			if (!(slot in c) || typeof c[slot] != "table") continue;
			local keys = [];
			foreach( k, _ in c[slot] ) keys.push(k);
			keys.sort();
			foreach( k in keys ) ::logInfo("PROBE " + kind + " " + _label + "." + k + " = property");
		}
		if (("__getTable" in c) && typeof c.__getTable == "table")
		{
			local writable = ("__setTable" in c) && typeof c.__setTable == "table" ? c.__setTable : {};
			local keys = [];
			foreach( k, _ in c.__getTable ) if (!(k in writable)) keys.push(k);
			keys.sort();
			foreach( k in keys ) ::logInfo("PROBE readonly " + _label + "." + k + " = property");
		}
	}
};

// The world.
local player = ::World.State.getPlayer();
local settlement = ::World.EntityManager.getSettlements()[0];
local nav = ::World.getNavigator();
report("world_tile", @() ::World.getTileSquare(20, 20));
report("world_navigator", @() nav);
report("world_navigator_settings", @() nav.createSettings());
report("world_time", @() ::World.getTime());
report("world_sprite", @() player.getSprite("body"));
report("world_brush", @() player.getSprite("body").getBrush());
report("world_label", @() settlement.getLabel("name"));
report("location", @() ::World.EntityManager.getLocations()[0]);
report("world_detail", @() ::World.getTileSquare(20, 20).spawnDetail("world_ship_01", ::Const.World.ZLevel.Object, 0));
report("world_camera", @() ::World.getCamera());
report("world_weather", @() ::World.getWeather());
report("world_clouds", @() ::World.getWeather().createCloudSettings());
report("world_birds", @() ::World.getWeather().createBirdSettings());
report("js_handle", @() ::World.State.m.WorldScreen.m.JSHandle);
report("gameplay_settings", @() ::Settings.getGameplaySettings());
report("temp_gameplay_settings", @() ::Settings.getTempGameplaySettings());
report("video_mode", @() ::Settings.getVideoMode());
report("control_settings", @() ::Settings.getControlSettings());

// The save stream and its metadata, through the flags' onSerialize.
local flags = ::World.Flags;
local onSerialize = flags.onSerialize;
flags.onSerialize = function ( _out )
{
	report("save_stream", @() _out);
	report("save_metadata", @() _out.getMetaData());
	return onSerialize(_out);
};
::World.State.m.IsAutosaving = false;
try { ::World.State.saveCampaign("bb_probe_natives"); } catch (e) { ::logInfo("PROBE native save = ERROR " + e); }
flags.onSerialize = onSerialize;
::PersistenceManager.deleteStorage("bb_probe_natives");

// Then a battle against a few bandits, once the intro event has been answered.
::BbProbe.DeferDone = true;
::__nativesBattle <- {
	Waited = 0,
	function answerIntro( _ )
	{
		local screen = ::World.State.getEventScreen();
		if (screen != null && screen.isVisible()) screen.onButtonPressed(0);
		::Time.scheduleEvent(::TimeUnit.Real, 2500, ::__nativesBattle.start, null);
	}
	function start( _ )
	{
		// As the UI payloads' walk starts its battle (it works in the original's probes).
		local st = ::World.State;
		local p = st.getLocalCombatProperties(st.getPlayer().getPos());
		p.CombatID = "Event";
		p.IsAutoAssigningBases = false;
		p.Entities = [];
		::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditDefenders, 100, ::Const.Faction.Enemy);
		st.startScriptedCombat(p, false, false, true);
		::Time.scheduleEvent(::TimeUnit.Real, 250, ::__nativesBattle.wait, null);
	}
	function enemies()
	{
		if (!("Entities" in ::Tactical) || ::Tactical.Entities == null) return [];
		return ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Enemy);
	}
	function wait( _ )
	{
		local e = ::__nativesBattle.enemies();
		local up = ("State" in ::Tactical) && ::Tactical.State != null && e.len() != 0 && e[0].isPlacedOnMap();
		if (!up && ++::__nativesBattle.Waited < 240)
		{
			::Time.scheduleEvent(::TimeUnit.Real, 250, ::__nativesBattle.wait, null);
			return;
		}
		try
		{
			if (!up) throw "the battle never began";
			::__nativesBattle.inBattle(e[0]);
		}
		catch( err )
		{
			::logInfo("PROBE native battle = ERROR " + err);
		}
		::BbProbe.done();
	}
	function inBattle( _enemy )
	{
		local report = function ( _label, _f )
		{
			try { local o = _f(); ::__nativeReport(_label, o); ::__propReport(_label, o); }
			catch( e ) { ::logInfo("PROBE native " + _label + " = ERROR " + e); }
		};
		local tile = _enemy.getTile();
		report("tactical_tile", @() tile);
		report("tactical_detail", @() tile.spawnDetail("bust_spider_gut", ::Const.Tactical.DetailFlag.Corpse, false));
		report("actor", @() _enemy);
		report("actor_sprite", @() _enemy.getSprite("body"));
		report("actor_brush", @() _enemy.getSprite("body").getBrush());
		report("tactical_camera", @() ::Tactical.getCamera());
		report("tactical_navigator", @() ::Tactical.getNavigator());
		report("tactical_navigator_settings", @() ::Tactical.getNavigator().createSettings());
		report("tactical_shaker", @() ::Tactical.getShaker());
		report("tactical_highlighter", @() ::Tactical.getHighlighter());
		report("tactical_weather", @() ::Tactical.getWeather());
		report("tactical_clouds", @() ::Tactical.getWeather().createCloudSettings());
		report("tactical_rain", @() ::Tactical.getWeather().createRainSettings());
		report("head_effect", @() ::Tactical.spawnHeadEffect(tile, [ "bust_head_01" ], ::createVec(-20, 15), -90.0, "")[0]);
		report("tactical_js_handle", @() ::Tactical.State.m.TacticalScreen.m.JSHandle);
	}
};
::Time.scheduleEvent(::TimeUnit.Real, 2500, ::__nativesBattle.answerIntro, null);
