// mode: campaign seed=rust-engine-test
// Fog of war: discovered tiles after the campaign starts and after uncovering more.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local function discovered()
{
	local size = ::World.getMapSize(), n = 0;
	for (local x = 0; x < size.X; x++)
		for (local y = 0; y < size.Y; y++)
			if (::World.getTileSquare(x, y).IsDiscovered) n++;
	return n;
}
p("discovered at start", discovered);
::World.uncoverFogOfWar(::createVec(7000, 9000), 500.0);
p("after uncover 7000,9000 r500", discovered);
::World.uncoverFogOfWar(::createVec(2000, 3000), 1234.5);
p("after uncover 2000,3000 r1234.5", discovered);
