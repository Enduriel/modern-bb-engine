// Record every World.getNavigator().findPath call with the grid as it is at that call.
// Before each call: the whole grid the first time (`GRID w h`, then one `ROW` per row of
// `type` or `type:roadDirections:heat` cells), afterwards `TILE col row cell` for each
// tile that changed since the previous call. Then `SET id …` once per distinct settings
// and `NAV fromQ fromR toQ toR maxLen settingsId` (axial coordinates).
local nav = ::World.getNavigator();
local orig = nav.findPath;
local seen = {};
local snap = null;

local function cell( _t )
{
	local c = "" + _t.Type;
	if (_t.RoadDirections != 0 || _t.HeatFromSettlements != 0)
		c += ":" + _t.RoadDirections + ":" + _t.HeatFromSettlements;
	return c;
}

local function syncGrid()
{
	local size = ::World.getMapSize();
	local full = snap == null || snap.W != size.X || snap.H != size.Y;
	if (full)
	{
		snap = { W = size.X, H = size.Y, Type = [], Road = [], Heat = [] };
		::print("GRID " + size.X + " " + size.Y + "\n");
	}
	for (local y = 0; y < size.Y; y++)
	{
		local line = "ROW";
		for (local x = 0; x < size.X; x++)
		{
			local t = ::World.getTileSquare(x, y);
			if (full)
			{
				snap.Type.push(t.Type);
				snap.Road.push(t.RoadDirections);
				snap.Heat.push(t.HeatFromSettlements);
				line += " " + cell(t);
				continue;
			}
			local i = y * size.X + x;
			if (snap.Type[i] != t.Type || snap.Road[i] != t.RoadDirections || snap.Heat[i] != t.HeatFromSettlements)
			{
				snap.Type[i] = t.Type;
				snap.Road[i] = t.RoadDirections;
				snap.Heat[i] = t.HeatFromSettlements;
				::print("TILE " + x + " " + y + " " + cell(t) + "\n");
			}
		}
		if (full)
			::print(line + "\n");
	}
}

local function axial( _o )
{
	try { return [_o.X, _o.Y]; } catch (e) {}
	local t = _o.getTile();
	return [t.X, t.Y];
}

nav.findPath = function( _from, _to, _settings, _maxLen )
{
	syncGrid();
	local f = axial(_from), t = axial(_to);
	local g = function( k, d ) { return k in _settings ? _settings[k] : d; };
	local key = ::format("%.17g", g("RoadMult", 0.0).tofloat()) + " "
		+ ::format("%.17g", g("HeatCost", 0).tofloat()) + " "
		+ (g("StopAtRoad", false) ? 1 : 0) + " " + (g("RoadOnly", false) ? 1 : 0) + " ";
	foreach (i, c in _settings.ActionPointCosts)
		key += (i > 0 ? "," : "") + (typeof c == "float" ? ::format("%.17g", c) : c);
	if (!(key in seen))
	{
		seen[key] <- seen.len();
		::print("SET " + seen[key] + " " + key + "\n");
	}
	::print("NAV " + f[0] + " " + f[1] + " " + t[0] + " " + t[1] + " " + _maxLen + " " + seen[key] + "\n");
	return orig.call(nav, _from, _to, _settings, _maxLen);
};
