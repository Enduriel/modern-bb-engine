// mode: campaign seed=rust-engine-test
// World tile grid after worldgen: tile fields, neighbours, distances, type counts.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local size = ::World.getMapSize();
p("map size", @() size.X + "x" + size.Y);
local squares = [[0, 0], [10, 10], [70, 85], [139, 169], [35, 120], [100, 40], [71, 84], [5, 160]];
foreach (c in squares)
{
	local t = ::World.getTileSquare(c[0], c[1]);
	local k = "sq " + c[0] + "," + c[1] + " ";
	foreach (f in ["Type", "TacticalType", "Subregion", "Region", "RoadDirections", "RiverDirections",
		"HeatFromSettlements", "X", "Y", "HasRoad", "HasRiver", "IsDiscovered", "IsOccupied"])
		p(k + f, @() t[f]);
	p(k + "Coords", @() t.Coords.X + "," + t.Coords.Y);
	p(k + "SquareCoords", @() t.SquareCoords.X + "," + t.SquareCoords.Y);
	p(k + "Pos", @() t.Pos.X + "," + t.Pos.Y);
	// Tile IDs are addresses in the original; compare neighbours by coordinates.
	p(k + "neighbours", function() {
		local s = "";
		for (local d = 0; d < 6; d++)
			s += t.hasNextTile(d) ? t.getNextTile(d).SquareCoords.X + "," + t.getNextTile(d).SquareCoords.Y + " " : "- ";
		return s;
	});
	p(k + "ID stable and unique", @() t.ID == ::World.getTileSquare(c[0], c[1]).ID && t.ID != ::World.getTileSquare(1, 1).ID);
}
local pairs = [[[10, 10], [70, 85]], [[70, 85], [71, 84]], [[0, 0], [139, 169]], [[35, 120], [100, 40]]];
foreach (q in pairs)
{
	local a = ::World.getTileSquare(q[0][0], q[0][1]), b = ::World.getTileSquare(q[1][0], q[1][1]);
	local k = q[0][0] + "," + q[0][1] + "->" + q[1][0] + "," + q[1][1];
	p("distance " + k, @() a.getDistanceTo(b));
	p("direction " + k, @() a.getDirectionTo(b));
}
for (local type = 0; type <= 18; type++)
	p("tiles of type " + type, @() ::World.getNumOfTilesWithType([type]));
foreach (pos in [[0, 0], [1000.5, 2000.25], [7055, 9010], [14160, 16740]])
	p("worldToTile " + pos[0] + "," + pos[1], function() { local c = ::World.worldToTile(::createVec(pos[0], pos[1])); return c.X + "," + c.Y; });
p("tiles of types 2,3,4", @() ::World.getNumOfTilesWithType([2, 3, 4]));
