// mode: campaign seed=bbuitest
// UI payload goldens of the world (the scripts' side): every payload the
// scripts send to the UI through a handle (`asyncCall`, `call`) while the world is walked
// through the steps of bb-ui's world goldens (bb-ui/probes/world_golden.nut): the
// origin's intro event, its "Huzzah!", the character screen, the nearest town, leaving
// it.
//
// Runs right after world_state.init(), before the world's first update: the probe
// harness's campaign mode (`--seed bbuitest`) in the original, `tests/ui_payloads.rs`
// in ours. Between steps, the original's driver (world.js) waits 3 s; ours runs 3 s of
// frames. Both call the same two methods on the world screen:
//   probePayloadReady()  whether the intro event is up;
//   probePayloadStep()   log what was sent since the last step, take the next step;
//                         "next", or "done" after the last.
//
// Each step's payloads are logged as one line per leaf:
//   PROBE <step> <handle>.<function>#<n> <path> = <type> <value>
// Calls repeat every frame, so each distinct payload of a handle's function is logged
// once per step (#1, #2, ... in first-sent order). Tables are walked in sorted key
// order. Numeric ids (a key ending in "id", any case) become first-seen ordinals ("id#3"):
// entity ids and item instance ids aren't comparable between runs. So does the entity id
// in a portrait's image name ("tacticalentity(<content id>,<entity id>,...)"), whose
// content id becomes "rand": it comes from Math.rand() without arguments, the C
// runtime's unseeded generator, whose state depends on every earlier call in the process. '<' and '>' become
// {lt} and {gt} (the original's log.html would swallow them), and line breaks spaces.

// As bb-ui's world goldens: no autosave over the player's slot, and a pinned generator.
::World.State.m.IsAutosaving = false;
::Math.seedRandom(20261013);

::__payloads <- {
	Step = "start",
	Seen = {},
	Order = [],
	Ids = {},
	NextId = 0
};

::__payloadEscape <- function ( _s )
{
	if (_s.find("<") == null && _s.find(">") == null && _s.find("\n") == null && _s.find("\r") == null && _s.find("\t") == null)
	{
		return _s;
	}

	local out = "";

	foreach( c in _s )
	{
		if (c == '<')
		{
			out += "{lt}";
		}
		else if (c == '>')
		{
			out += "{gt}";
		}
		else if (c == '\n' || c == '\r' || c == '\t')
		{
			out += " ";
		}
		else
		{
			out += c.tochar();
		}
	}

	return out;
};

::__payloadId <- function ( _v )
{
	local key = typeof _v + ":" + _v;

	if (!(key in ::__payloads.Ids))
	{
		::__payloads.NextId += 1;
		::__payloads.Ids[key] <- "id#" + ::__payloads.NextId;
	}

	return ::__payloads.Ids[key];
};

::__payloadIsNumeric <- function ( _v )
{
	if (typeof _v == "integer")
	{
		return true;
	}

	if (typeof _v != "string" || _v.len() == 0)
	{
		return false;
	}

	foreach( i, c in _v )
	{
		if (!(c >= '0' && c <= '9' || c == '-' && i == 0))
		{
			return false;
		}
	}

	return true;
};

// "tacticalentity(<content id>,<entity id>,...)" and "worldentity(<random>,<entity id>)"
// image names, with "rand" for the first and the entity id as its ordinal.
::__payloadEntityImage <- function ( _s )
{
	if (_s.find("tacticalentity(") != 0 && _s.find("worldentity(") != 0)
	{
		return _s;
	}

	local first = _s.find(",");

	if (first == null)
	{
		return _s;
	}

	local second = _s.find(",", first + 1);

	if (second == null)
	{
		second = _s.find(")", first + 1);
	}

	if (second == null || !::__payloadIsNumeric(_s.slice(first + 1, second)))
	{
		return _s;
	}

	local open = _s.find("(");
	return _s.slice(0, open + 1) + "rand," + ::__payloadId(_s.slice(first + 1, second).tointeger()) + _s.slice(second);
};

::__payloadSerialize <- function ( _v, _path, _isId, _out )
{
	switch(typeof _v)
	{
	case "table":
		local keys = [];

		foreach( k, _ in _v )
		{
			keys.push(k);
		}

		if (keys.len() == 0)
		{
			_out.push(_path + " = table {}");
			return;
		}

		keys.sort(@(a, b) a.tostring() <=> b.tostring());

		foreach( k in keys )
		{
			local name = k.tostring();
			local isId = name.len() >= 2 && name.slice(name.len() - 2).tolower() == "id";
			::__payloadSerialize(_v[k], _path + "." + ::__payloadEscape(name), isId, _out);
		}

		return;

	case "array":
		if (_v.len() == 0)
		{
			_out.push(_path + " = array []");
			return;
		}

		foreach( i, e in _v )
		{
			::__payloadSerialize(e, _path + "[" + i + "]", _isId, _out);
		}

		return;

	case "string":
		if (_isId && ::__payloadIsNumeric(_v))
		{
			_out.push(_path + " = string " + ::__payloadId(_v));
		}
		else
		{
			_out.push(_path + " = string " + ::__payloadEscape(::__payloadEntityImage(_v)));
		}

		return;

	case "integer":
		_out.push(_path + " = integer " + (_isId ? ::__payloadId(_v) : _v));
		return;

	case "float":
	case "bool":
		_out.push(_path + " = " + typeof _v + " " + _v);
		return;

	case "null":
		_out.push(_path + " = null");
		return;

	default:
		_out.push(_path + " = " + typeof _v);
	}
};

::__payloadRecord <- function ( _handle, _function, _args )
{
	// The probe harness's own screen.
	if (_handle.find("BbProbe") == 0)
	{
		return;
	}

	local lines = [];

	foreach( i, a in _args )
	{
		::__payloadSerialize(a, "arg" + i, false, lines);
	}

	if (lines.len() == 0)
	{
		lines.push("(no data)");
	}

	local key = _handle + "." + _function;

	if (!(key in ::__payloads.Seen))
	{
		::__payloads.Seen[key] <- [];
		::__payloads.Order.push(key);
	}

	local text = lines.reduce(@(a, b) a + "\n" + b);

	foreach( known in ::__payloads.Seen[key] )
	{
		if (known.Text == text)
		{
			return;
		}
	}

	::__payloads.Seen[key].push({
		Text = text,
		Lines = lines
	});
};

// Log what was sent since the last step, under its name, and start a new step.
::__payloadFlush <- function ( _next )
{
	local keys = clone ::__payloads.Order;
	keys.sort();

	foreach( key in keys )
	{
		foreach( n, payload in ::__payloads.Seen[key] )
		{
			foreach( line in payload.Lines )
			{
				::logInfo("PROBE " + ::__payloads.Step + " " + key + "#" + (n + 1) + " " + line);
			}
		}
	}

	::__payloads.Step = _next;
	::__payloads.Seen = {};
	::__payloads.Order = [];
};

// A stand-in for a native UI handle that records what goes through it.
::__payloadHandle <- function ( _name, _real )
{
	if (_real == null || typeof _real == "table" && "PayloadName" in _real)
	{
		return _real;
	}

	return {
		Real = _real,
		PayloadName = _name,
		function asyncCall( _function, ... )
		{
			::__payloadRecord(this.PayloadName, "asyncCall:" + _function, vargv);

			if (vargv.len() == 0)
			{
				return this.Real.asyncCall(_function);
			}

			return this.Real.asyncCall(_function, vargv[0]);
		}

		function call( _function, ... )
		{
			::__payloadRecord(this.PayloadName, "call:" + _function, vargv);

			if (vargv.len() == 0)
			{
				return this.Real.call(_function);
			}

			return this.Real.call(_function, vargv[0]);
		}

		function connectToModule( _id, _owner )
		{
			return ::__payloadHandle(this.PayloadName + "/" + _id, this.Real.connectToModule(_id, _owner));
		}

	};
};

// New handles: UI.connect and UI.disconnect see the stand-ins.
{
	local ui = ::UI;
	local connect = ui.connect;
	local disconnect = ui.disconnect;
	local wrapped = {
		connect = function ( _name, _owner )
		{
			return ::__payloadHandle(_name, connect.call(ui, _name, _owner));
		},
		disconnect = function ( _handle )
		{
			local real = typeof _handle == "table" && "PayloadName" in _handle ? _handle.Real : _handle;
			return disconnect.call(ui, real);
		}
	};

	if (typeof ui == "table")
	{
		ui.connect = wrapped.connect;
		ui.disconnect = wrapped.disconnect;
	}
	else
	{
		// A native object: stand in for it, forwarding everything else.
		wrapped.setdelegate({
			function _get( _key )
			{
				local v = ui[_key];

				if (typeof v == "function" || typeof v == "native function")
				{
					return function ( ... )
					{
						local args = [
							ui
						];
						args.extend(vargv);
						return v.acall(args);
					};
				}

				return v;
			}

		});
		::UI <- wrapped;
	}
}

// Existing handles: the world state's screens and their modules, named after the member
// that holds them.
{
	local visited = {};
	local wrap;
	wrap = function ( _object, _name, _depth )
	{
		if (_depth > 4 || _object in visited || !("m" in _object) || typeof _object.m != "table")
		{
			return;
		}

		visited[_object] <- true;

		if ("JSHandle" in _object.m && _object.m.JSHandle != null)
		{
			_object.m.JSHandle = ::__payloadHandle(_name, _object.m.JSHandle);
		}

		foreach( k, v in _object.m )
		{
			if (typeof v == "table" && k != "Parent")
			{
				wrap(v, k, _depth + 1);
			}
		}
	};
	// World.State itself may be a native object: start from its members.
	foreach( k, v in ::World.State.m )
	{
		if (typeof v == "table")
		{
			wrap(v, k, 1);
		}
	}
}

::World.State.m.WorldScreen.probePayloadReady <- function ( _unused = null )
{
	return ::World.State.m.EventScreen.isVisible();
};

::__payloadSteps <- [
	"huzzah",
	"character",
	"closeCharacter",
	"town",
	"leaveTown"
];
::World.State.m.WorldScreen.probePayloadStep <- function ( _unused = null )
{
	local state = ::World.State;

	if (::__payloadSteps.len() == 0)
	{
		::__payloadFlush("done");
		return "done";
	}

	local step = ::__payloadSteps.remove(0);
	::__payloadFlush(step);

	if (step == "huzzah")
	{
		// The intro event's only button, as its page would report the click.
		state.m.EventScreen.onButtonPressed(0);
	}
	else if (step == "character")
	{
		// The top bar's Brothers button.
		state.topbar_options_module_onBrothersButtonClicked();
	}
	else if (step == "closeCharacter")
	{
		state.character_screen_onClosePressed();
	}
	else if (step == "town")
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

		::logInfo("PROBE town nearest = string " + nearest.getName() + " at " + distance);
		state.enterLocation(nearest);
	}
	else if (step == "leaveTown")
	{
		state.town_screen_main_dialog_module_onLeaveButtonClicked();
	}

	return "next";
};
::logInfo("PROBE_SECTION ui_payloads/world");
