// mode: campaign seed=rust-engine-test
// WorldTile.getDirectionTo / getDirection8To from one tile to every tile within 9 of it
// (one row per square column), and to distant tiles.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local center = ::World.getTileSquare(70, 85);
for (local dx = -9; dx <= 9; dx++)
{
	p("dir row " + dx, function() {
		local s = "";
		for (local dy = -9; dy <= 9; dy++)
		{
			local t = ::World.getTileSquare(70 + dx, 85 + dy);
			s += center.getDirectionTo(t) + "" + center.getDirection8To(t) + " ";
		}
		return s;
	});
}
foreach (c in [[70, 20], [110, 40], [130, 85], [110, 130], [70, 150], [30, 130], [10, 85], [30, 40], [72, 160], [139, 0], [0, 169]])
	p("dir far " + c[0] + "," + c[1], function() { local t = ::World.getTileSquare(c[0], c[1]); return center.getDirectionTo(t) + "/" + center.getDirection8To(t); });
