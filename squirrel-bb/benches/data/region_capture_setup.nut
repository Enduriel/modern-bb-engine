// Record every World.findNextRegion call with the grid as it is at that call.
// buildLabels changes a tile's Region between calls in only two ways: it sets the queried
// tile's own Region after each call, and it reassigns regions in bulk between its passes
// (which use different distances). So the capture writes:
// * at the first call and whenever the distance changes: the whole grid (`GRID w h`, then
//   one `ROW` per row of `type` or `type:region` cells) and `PASS dist`;
// * otherwise, `REGION col row region` for the previous call's tile if its Region is not
//   the value that call returned;
// * per call, `F col row result [type]` (square coordinates, the result the engine
//   returned; `type` only when it is not the tile's own Type).
local orig = ::World.findNextRegion;
local last = null;

::World.findNextRegion = function( _tile, _dist, _type )
{
	if (last == null || last.Dist != _dist)
	{
		local size = ::World.getMapSize();
		::print("GRID " + size.X + " " + size.Y + "\n");
		for (local y = 0; y < size.Y; y++)
		{
			local line = "ROW";
			for (local x = 0; x < size.X; x++)
			{
				local t = ::World.getTileSquare(x, y);
				line += " " + t.Type + (t.Region != 0 ? ":" + t.Region : "");
			}
			::print(line + "\n");
		}
		::print("PASS " + _dist + "\n");
	}
	else if (last.Tile.Region != last.Result)
	{
		local c = last.Tile.SquareCoords;
		::print("REGION " + c.X + " " + c.Y + " " + last.Tile.Region + "\n");
	}
	local result = orig(_tile, _dist, _type);
	local c = _tile.SquareCoords;
	::print("F " + c.X + " " + c.Y + " " + result + (_type != _tile.Type ? " " + _type : "") + "\n");
	last = { Tile = _tile, Dist = _dist, Result = result };
	return result;
};
