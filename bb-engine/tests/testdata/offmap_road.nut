// A tactical.plains battle whose north-south road walks off the map's west edge, as
// vanilla's addRoads can (its left step tests roadY, not roadX): Math.rand is steered
// during addRoads so the walk reaches roadX = -1 and then runs south to the end. Used by
// tests/tactical_offmap_road.rs. Records what the road's getTileSquare calls returned
// off the map in ::__offmap.

::__offmap <- { Reads = [], InRoadFill = 0, AddRoads = 0 };

local plains = ::MapGen.get("tactical.plains");
local road = ::MapGen.get("tactical.tile.road");
local origAdd = plains.addRoads;
local origFill = road.fill;
road.fill <- function ( _rect, _props, _pass = 1 )
{
	if (_rect.X < 0)
	{
		local t = ::Tactical.getTileSquare(_rect.X, _rect.Y);
		::__offmap.Reads.push(t == null ? "null" : t.SquareCoords.X + "," + t.SquareCoords.Y);
	}
	::__offmap.InRoadFill++;
	try { origFill.call(this, _rect, _props, _pass); }
	catch (e) { ::__offmap.InRoadFill--; throw e; }
	::__offmap.InRoadFill--;
};
plains.addRoads <- function ( _rect, _props )
{
	::__offmap.AddRoads++;
	local fake = { Tile = { RoadDirections = ::Const.DirectionBit[::Const.Direction.N] | ::Const.DirectionBit[::Const.Direction.S] } };
	local rand = ::Math.rand;
	local sim = { X = _rect.W / 2, Y = 0, Moved = false };
	::Math.rand = function ( _a, _b )
	{
		if (_a != 0 || _b != 100) return rand(_a, _b);
		// The road tile's own draw (a cartwheel at 1%): none.
		if (::__offmap.InRoadFill > 0) return 99;
		// The walk: south (r < 50) until it may step west, west (r >= 66) to -1, then south.
		if (!sim.Moved) { sim.Y++; sim.Moved = true; return 0; }
		if (sim.Y >= 2 && sim.X > -1) { sim.X--; sim.Moved = false; return 99; }
		sim.Y++;
		return 0;
	};
	try { origAdd.call(this, _rect, fake); }
	catch (e) { ::Math.rand = rand; throw e; }
	::Math.rand = rand;
};

local st = ::World.State;
local p = st.getLocalCombatProperties(st.getPlayer().getPos());
p.CombatID = "Event";
p.IsAutoAssigningBases = false;
p.TerrainTemplate = "tactical.plains";
p.LocationTemplate = null;
p.Entities = [];
::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditDefenders, 100, ::Const.Faction.Enemy);
st.startScriptedCombat(p, false, false, true);
