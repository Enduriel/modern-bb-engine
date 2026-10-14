// mode: menu
// compare: subset
// Coverage: every native function the scripts can reach from the root table, at the main
// menu. For each global (and the root itself), the natives on it, its delegate chain
// (tables) or its class and base classes (instances, classes), by name. Where a native
// lives differs between the engines, so only the names reachable from each global count,
// and only the original's must exist in ours (our extras are fine).

// The instances' own `getclass`, taken from a plain instance: an object whose class has
// a `_get` metamethod would answer `obj.getclass` through it.
::__nativeGetClass <- (class {})().getclass;
::__nativeIsNative <- function ( _v )
{
	return (typeof _v == "function" || typeof _v == "native function") && _v.getinfos().native;
};

::__nativeNames <- function ( _obj )
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
	return out;
};

::__nativeReport <- function ( _label, _obj )
{
	foreach( n in ::__nativeNames(_obj) )
	{
		::logInfo("PROBE native " + _label + "." + n + " = native");
	}
};

::__nativeReport("root", ::getroottable());
local globals = [];

foreach( k, v in ::getroottable() )
{
	if (typeof k == "string" && k.find("__native") != 0 && (typeof v == "table" || typeof v == "instance" || typeof v == "class"))
	{
		globals.push(k);
	}
}

globals.sort();

foreach( k in globals )
{
	::__nativeReport(k, ::getroottable()[k]);
}

::__nativeReport("vec", ::createVec(1, 2));
::__nativeReport("color", ::createColor("#ffffff"));
