// mode: campaign seed=rust-engine-test
// World.findNextTileOfType for distances 1-8 around several tiles, with the local type
// layout so the original's candidate rule can be read off.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
foreach (c in [[70, 85], [35, 120], [100, 40], [120, 150], [60, 30]])
{
	local center = ::World.getTileSquare(c[0], c[1]);
	local k = c[0] + "," + c[1];
	foreach (type in [1, 2, 4, 9, 12])
		p("nextTileOfType " + k + " t" + type, function() {
			local s = "";
			for (local d = 1; d <= 8; d++)
			{
				local r = {};
				s += ::World.findNextTileOfType(center, d, type, r) ? r.Result.SquareCoords.X + "," + r.Result.SquareCoords.Y + " " : "- ";
			}
			return s;
		});
	for (local dx = -8; dx <= 8; dx++)
		p("types " + k + " col " + dx, function() {
			local s = "";
			for (local dy = -8; dy <= 8; dy++)
			{
				local x = c[0] + dx, y = c[1] + dy;
				s += ::World.isValidTileSquare(x, y) ? ::World.getTileSquare(x, y).Type + (::World.getTileSquare(x, y).IsOccupied ? "o" : "") + " " : "- ";
			}
			return s;
		});
}
