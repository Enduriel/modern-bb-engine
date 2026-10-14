// mode: campaign seed=rust-engine-test
// compare: subset
// Coverage: the natives on the objects the world hands out (menu/natives.nut defines the
// helpers when both run in one launch; this redefines them so it runs alone too).

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
		::__nativeReport(_label, _f());
	}
	catch( e )
	{
		::logInfo("PROBE native " + _label + " = ERROR " + e);
	}
}

local nav = ::World.getNavigator();
local settlements = ::World.EntityManager.getSettlements();
report("tile", @() ::World.getTileSquare(20, 20));
report("navigator", @() nav);
report("navigator_settings", @() nav.createSettings());
report("path", @() nav.findPath(settlements[0].getTile(), settlements[1].getTile(), nav.createSettings(), 0));
report("settlement", @() settlements[0]);
report("player_party", @() ::World.State.getPlayer());
report("world_time", @() ::World.getTime());
report("player_roster", @() ::World.getPlayerRoster());
report("brother", @() ::World.getPlayerRoster().getAll()[0]);
report("world_state", @() ::World.State);
