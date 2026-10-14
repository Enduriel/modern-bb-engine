// mode: campaign seed=rust-engine-test
// World.findNextRegion / findNextTileOfType after worldgen.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
foreach (c in [[70, 85], [35, 120], [100, 40], [10, 10], [120, 150], [60, 30]])
{
	local t = ::World.getTileSquare(c[0], c[1]);
	local k = c[0] + "," + c[1];
	p("findNextRegion " + k + " d3 any", @() ::World.findNextRegion(t, 3, 0));
	p("findNextRegion " + k + " d16 any", @() ::World.findNextRegion(t, 16, 0));
	p("findNextRegion " + k + " d5 own type", @() ::World.findNextRegion(t, 5, t.Type));
	foreach (type in [1, 2, 4, 9, 12])
		p("findNextTileOfType " + k + " d8 t" + type, function() {
			local r = {};
			local found = ::World.findNextTileOfType(t, 8, type, r);
			return found ? r.Result.SquareCoords.X + "," + r.Result.SquareCoords.Y : "none";
		});
}
